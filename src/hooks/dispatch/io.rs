use super::Advice;
use crate::lifecycle::Scope;
use std::path::Path;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::io::{ErrorKind, Read};
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    struct Nonblocking {
        fd: RawFd,
        previous: i32,
    }
    impl Nonblocking {
        fn new(fd: RawFd) -> std::result::Result<Self, ()> {
            // SAFETY: fcntl changes flags on a live borrowed descriptor only.
            let previous = unsafe { libc::fcntl(fd, libc::F_GETFL) };
            if previous < 0
                || unsafe { libc::fcntl(fd, libc::F_SETFL, previous | libc::O_NONBLOCK) } < 0
            {
                return Err(());
            }
            Ok(Self { fd, previous })
        }
    }
    impl Drop for Nonblocking {
        fn drop(&mut self) {
            // SAFETY: the owner outlives this guard.
            unsafe {
                libc::fcntl(self.fd, libc::F_SETFL, self.previous);
            }
        }
    }
    pub(super) fn input(limit: usize) -> std::result::Result<Vec<u8>, ()> {
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        let _flags = Nonblocking::new(input.as_raw_fd())?;
        let deadline = Instant::now() + Duration::from_millis(750);
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            if Instant::now() >= deadline {
                return Err(());
            }
            match input.read(&mut buffer) {
                Ok(0) => return Ok(bytes),
                Ok(n) if bytes.len() + n <= limit => bytes.extend_from_slice(&buffer[..n]),
                Ok(_) => return Err(()),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => return Err(()),
            }
        }
    }
    struct Process(Child);
    impl Drop for Process {
        fn drop(&mut self) {
            // SAFETY: child starts its own process group; nested Git probes inherit it.
            unsafe {
                libc::kill(-(self.0.id() as i32), libc::SIGKILL);
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    pub(super) fn observe(
        root: &Path,
        scope: Scope,
        audience: &str,
        session: &str,
        claimed: Option<&str>,
        registration: bool,
        force_notice: bool,
    ) -> std::result::Result<Advice, ()> {
        let executable = std::env::current_exe().map_err(|_| ())?;
        let mut command = Command::new(executable);
        command
            .arg("--root")
            .arg(root)
            .arg("hook-check")
            .arg("--scope")
            .arg(scope.name())
            .arg("--audience")
            .arg(audience)
            .arg("--session-scope")
            .arg(session)
            .env("ASTRAL_HOOK_DEPTH", "1")
            .env("ASTRAL_HOOK_CHILD", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        if let Some(claimed) = claimed {
            command.arg("--claimed-session").arg(claimed);
        }
        if registration {
            command.arg("--require-git-registration");
        }
        if force_notice {
            command.arg("--force-notice");
        }
        let mut child = Process(command.spawn().map_err(|_| ())?);
        let mut output = child.0.stdout.take().ok_or(())?;
        let _flags = Nonblocking::new(output.as_raw_fd())?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut bytes = Vec::new();
        let mut buffer = [0; 2048];
        let mut eof = false;
        loop {
            if Instant::now() >= deadline {
                return Err(());
            }
            if !eof {
                match output.read(&mut buffer) {
                    Ok(0) => eof = true,
                    Ok(n) if bytes.len() + n <= 8192 => bytes.extend_from_slice(&buffer[..n]),
                    Ok(_) => return Err(()),
                    Err(e)
                        if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) => {}
                    Err(_) => return Err(()),
                }
            }
            if let Some(status) = child.0.try_wait().map_err(|_| ())? {
                if !status.success() {
                    return Err(());
                }
                if eof {
                    return serde_json::from_slice(&bytes).map_err(|_| ());
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

pub(super) fn input(limit: usize) -> std::result::Result<Vec<u8>, ()> {
    #[cfg(unix)]
    {
        unix::input(limit)
    }
    #[cfg(not(unix))]
    {
        let _ = limit;
        Err(())
    }
}
pub(super) fn observe(
    root: &Path,
    scope: Scope,
    audience: &str,
    session: &str,
    claimed: Option<&str>,
    registration: bool,
    force_notice: bool,
) -> std::result::Result<Advice, ()> {
    #[cfg(unix)]
    {
        unix::observe(
            root,
            scope,
            audience,
            session,
            claimed,
            registration,
            force_notice,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = (
            root,
            scope,
            audience,
            session,
            claimed,
            registration,
            force_notice,
        );
        Err(())
    }
}
