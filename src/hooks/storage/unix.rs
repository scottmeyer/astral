use super::*;
use std::ffi::CString;
use std::fs::{File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

pub(crate) struct Directory {
    file: File,
    pub path: PathBuf,
}
pub(crate) struct Guard(File);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}
fn secure(meta: &Metadata, directory: bool, private: bool) -> Result<()> {
    if meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o7022 != 0
        || if directory {
            !meta.is_dir()
        } else {
            !meta.is_file() || meta.nlink() != 1
        }
        || (private && meta.mode() & 0o7777 != if directory { 0o700 } else { 0o600 })
    {
        return Err(error(
            "HOOK_STORAGE_UNSAFE",
            "hook state requires owned, non-writable-by-others regular files and directories",
        ));
    }
    Ok(())
}
fn identity(meta: &Metadata) -> String {
    format!("{}:{}", meta.dev(), meta.ino())
}
fn stamp(meta: &Metadata) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}:{}",
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mode(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec()
    )
}
fn cname(value: &str) -> Result<CString> {
    name(value)?;
    CString::new(value).map_err(|_| error("HOOK_STORAGE_PATH", "invalid entry name"))
}
impl Directory {
    /// Lock an existing project directory without adding private lock metadata.
    pub fn lock_directory(&self) -> Result<Guard> {
        let file = self
            .file
            .try_clone()
            .map_err(|_| error("HOOK_STORAGE_IO", "cannot duplicate directory"))?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|_| {
            error(
                "HOOK_STORAGE_BUSY",
                "directory has another cooperating writer",
            )
        })?;
        Ok(Guard(file))
    }
    pub fn open(path: &Path, private: bool) -> Result<Self> {
        if !path.is_absolute() || path.as_os_str().len() > 4096 {
            return Err(error(
                "HOOK_PATH",
                "hook directory must be a bounded absolute path",
            ));
        }
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| error("HOOK_STORAGE_UNAVAILABLE", "cannot open hook directory"))?;
        secure(
            &file
                .metadata()
                .map_err(|_| error("HOOK_STORAGE_IO", "cannot inspect hook directory"))?,
            true,
            private,
        )?;
        Ok(Self {
            file,
            path: path.into(),
        })
    }
    pub fn identity(&self) -> Result<String> {
        Ok(identity(&self.file.metadata().map_err(|_| {
            error("HOOK_STORAGE_IO", "cannot inspect directory")
        })?))
    }
    pub fn unchanged(&self) -> Result<()> {
        let now = Self::open(&self.path, false)?;
        if now.identity()? != self.identity()? {
            return Err(error("HOOK_CHANGED", "hook directory association changed"));
        }
        Ok(())
    }
    pub fn child(&self, child: &str, create: bool, private: bool) -> Result<Option<Self>> {
        let component = cname(child)?;
        if create
            && unsafe {
                libc::mkdirat(
                    self.file.as_raw_fd(),
                    component.as_ptr(),
                    if private { 0o700 } else { 0o755 },
                )
            } != 0
        {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error("HOOK_STORAGE_IO", "cannot create hook directory"));
            }
        }
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error(
                "HOOK_STORAGE_UNSAFE",
                "hook directory is unsafe or unavailable",
            ));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        secure(
            &file
                .metadata()
                .map_err(|_| error("HOOK_STORAGE_IO", "cannot inspect directory"))?,
            true,
            private,
        )?;
        self.unchanged()?;
        Ok(Some(Self {
            file,
            path: self.path.join(child),
        }))
    }
    pub fn read(&self, entry: &str, limit: usize, private: bool) -> Result<Option<Snapshot>> {
        if limit > MAX_BYTES {
            return Err(error("HOOK_LIMIT", "hook file limit is too large"));
        }
        let component = cname(entry)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                return Ok(None);
            }
            return Err(error(
                "HOOK_STORAGE_UNSAFE",
                "hook file is unsafe or unavailable",
            ));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        let before = file
            .metadata()
            .map_err(|_| error("HOOK_STORAGE_IO", "cannot inspect hook file"))?;
        secure(&before, false, private)?;
        if before.len() > limit as u64 {
            return Err(error("HOOK_LIMIT", "hook file exceeds its byte limit"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| error("HOOK_STORAGE_IO", "cannot read hook file"))?;
        let after = file
            .metadata()
            .map_err(|_| error("HOOK_STORAGE_IO", "cannot inspect hook file"))?;
        secure(&after, false, private)?;
        if bytes.len() > limit
            || bytes.len() as u64 != after.len()
            || stamp(&before) != stamp(&after)
        {
            return Err(error("HOOK_CHANGED", "hook file changed while read"));
        }
        let named_fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                component.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if named_fd < 0 {
            return Err(error(
                "HOOK_CHANGED",
                "hook file association changed while read",
            ));
        }
        let named = unsafe { File::from_raw_fd(named_fd) };
        let named_metadata = named
            .metadata()
            .map_err(|_| error("HOOK_STORAGE_IO", "cannot inspect hook association"))?;
        secure(&named_metadata, false, private)?;
        if stamp(&after) != stamp(&named_metadata) {
            return Err(error(
                "HOOK_CHANGED",
                "hook file association changed while read",
            ));
        }
        self.unchanged()?;
        Ok(Some(Snapshot {
            sha256: crate::hash(&bytes),
            bytes,
            identity: stamp(&after),
            mode: after.mode() & 0o7777,
        }))
    }
    pub fn lock(&self) -> Result<Guard> {
        let entry = c"owner.lock";
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                entry.as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK
                    | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(error(
                "HOOK_STORAGE_LOCK",
                "cannot open advisory storage lock",
            ));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        secure(
            &file
                .metadata()
                .map_err(|_| error("HOOK_STORAGE_IO", "cannot inspect lock"))?,
            false,
            true,
        )?;
        fs2::FileExt::try_lock_exclusive(&file)
            .map_err(|_| error("HOOK_STORAGE_BUSY", "hook metadata has another owner"))?;
        Ok(Guard(file))
    }
    /// The caller holds the private operation lock. Expected bytes are rechecked
    /// immediately before publication; ordinary noncooperating writers must stop.
    pub fn write(
        &self,
        entry: &str,
        bytes: &[u8],
        expected: Option<&str>,
        mode: u32,
    ) -> Result<()> {
        if bytes.len() > MAX_BYTES || (mode > 0o777 || mode & 0o022 != 0) {
            return Err(error("HOOK_LIMIT", "invalid hook publication size or mode"));
        }
        self.unchanged()?;
        let component = cname(entry)?;
        let current = self.read(entry, MAX_BYTES, false)?;
        if current.as_ref().map(|s| s.sha256.as_str()) != expected {
            return Err(error(
                "HOOK_CHANGED",
                "hook file differs from reviewed bytes",
            ));
        }
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| error("HOOK_ENTROPY", "OS randomness unavailable"))?;
        let temporary = format!(
            "pending-{}",
            random
                .iter()
                .map(|v| format!("{v:02x}"))
                .collect::<String>()
        );
        let temp = cname(&temporary)?;
        let fd = unsafe {
            libc::openat(
                self.file.as_raw_fd(),
                temp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                mode,
            )
        };
        if fd < 0 {
            return Err(error("HOOK_STORAGE_IO", "cannot stage hook publication"));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        // mode_t is u16 on Darwin and u32 on Linux; mode is bounded to 0777 above.
        #[allow(clippy::unnecessary_cast)]
        let file_mode = mode as libc::mode_t;
        if unsafe { libc::fchmod(file.as_raw_fd(), file_mode) } != 0 {
            return Err(error(
                "HOOK_STORAGE_IO",
                "cannot set hook file mode; staging file retained",
            ));
        }
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| {
                error(
                    "HOOK_STORAGE_IO",
                    "cannot persist hook publication; staging file retained",
                )
            })?;
        self.unchanged()?;
        let latest = self.read(entry, MAX_BYTES, false)?;
        if latest.as_ref().map(|s| (&s.sha256, &s.identity))
            != current.as_ref().map(|s| (&s.sha256, &s.identity))
        {
            return Err(error(
                "HOOK_CHANGED",
                "hook file changed before publication; staging file retained",
            ));
        }
        let result = if expected.is_none() {
            #[cfg(target_os = "linux")]
            {
                unsafe {
                    libc::renameat2(
                        self.file.as_raw_fd(),
                        temp.as_ptr(),
                        self.file.as_raw_fd(),
                        component.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                }
            }
            #[cfg(target_os = "macos")]
            {
                unsafe {
                    libc::renameatx_np(
                        self.file.as_raw_fd(),
                        temp.as_ptr(),
                        self.file.as_raw_fd(),
                        component.as_ptr(),
                        libc::RENAME_EXCL,
                    )
                }
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                return Err(error(
                    "HOOK_UNSUPPORTED_PLATFORM",
                    "atomic hook installation requires Linux or macOS",
                ));
            }
        } else {
            unsafe {
                libc::renameat(
                    self.file.as_raw_fd(),
                    temp.as_ptr(),
                    self.file.as_raw_fd(),
                    component.as_ptr(),
                )
            }
        };
        if result != 0 {
            return Err(error(
                "HOOK_CHANGED",
                "hook publication failed; staging file retained",
            ));
        }
        self.file
            .sync_all()
            .map_err(|_| error("HOOK_STORAGE_IO", "cannot persist hook directory"))?;
        Ok(())
    }
}
