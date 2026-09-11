#![cfg(unix)]

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Callback {
    child: Child,
    checker_group: Option<i32>,
}

impl Drop for Callback {
    fn drop(&mut self) {
        // Both groups were created solely by this test's callback/observer.
        // The observer group is learned while its fixture Git process is alive.
        unsafe {
            if let Some(group) = self.checker_group.take() {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        if self.child.try_wait().ok().flatten().is_none() {
            // SAFETY: this still-owned child created its own process group.
            unsafe {
                libc::kill(-(self.child.id() as i32), libc::SIGKILL);
            }
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn pid(path: &Path) -> Option<i32> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn running(pid: i32) -> bool {
    // A dead process may briefly remain a zombie until the OS reaps it. It can
    // no longer execute or retain descendants, so do not call it a running leak.
    let output = Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "stat="])
        .output()
        .expect("inspect fixture process state");
    let state = String::from_utf8_lossy(&output.stdout);
    output.status.success() && !state.trim().is_empty() && !state.trim().starts_with('Z')
}

#[test]
fn codex_callback_deadline_terminates_git_and_its_descendant() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().canonicalize().unwrap();
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let git = bin.join("git");
    fs::write(
        &git,
        "#!/bin/sh\nprintf '%s\\n' \"$$\" > \"$ASTRAL_HOOK_TEST_GIT_PID\"\n/bin/sh -c 'printf \"%s\\n\" \"$$\" > \"$ASTRAL_HOOK_TEST_DESCENDANT_PID\"; exec /bin/sleep 60' &\nwait\n",
    ).unwrap();
    fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
    let git_pid_file = root.join("git.pid");
    let descendant_pid_file = root.join("descendant.pid");
    let started = Instant::now();
    let child = Command::new(env!("CARGO_BIN_EXE_astral"))
        .args(["hook", "codex"])
        .current_dir(&root)
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .env("ASTRAL_HOOK_TEST_GIT_PID", &git_pid_file)
        .env("ASTRAL_HOOK_TEST_DESCENDANT_PID", &descendant_pid_file)
        .env_remove("ASTRAL_HOOK_DEPTH")
        .env_remove("ASTRAL_HOOK_CHILD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut callback = Callback {
        child,
        checker_group: None,
    };
    callback
        .child
        .stdin
        .take()
        .unwrap()
        .write_all(
            &serde_json::to_vec(&json!({
                "hook_event_name":"SessionStart", "source":"startup",
                "session_id":"01a10000-1234-7000-8000-000000000001",
                "cwd":root, "model":"gpt-6-astra"
            }))
            .unwrap(),
        )
        .unwrap();

    let (git_pid, descendant_pid) = loop {
        if let (Some(git), Some(descendant)) = (pid(&git_pid_file), pid(&descendant_pid_file)) {
            // SAFETY: getpgid inspects only the newly written live fixture PID.
            let group = unsafe { libc::getpgid(git) };
            assert!(group > 1);
            assert_ne!(group, unsafe { libc::getpgrp() });
            callback.checker_group = Some(group);
            break (git, descendant);
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "fixture Git did not spawn its descendant"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(running(git_pid));
    assert!(running(descendant_pid));
    let status = loop {
        if let Some(status) = callback.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < Duration::from_secs(7),
            "callback exceeded its deadline"
        );
        thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "advisory failures must exit successfully");
    assert!(started.elapsed() < Duration::from_secs(7));
    let mut stdout = String::new();
    callback
        .child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut stdout)
        .unwrap();
    let response: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        response,
        json!({"hookSpecificOutput": {
            "hookEventName":"SessionStart",
            "additionalContext":"Astral lifecycle observation unavailable. Run astral lifecycle check to inspect current context; no worker state was changed."
        }})
    );

    let cleanup_deadline = Instant::now() + Duration::from_secs(2);
    while running(git_pid) || running(descendant_pid) {
        assert!(
            Instant::now() < cleanup_deadline,
            "a timed-out observer left a running descendant"
        );
        thread::sleep(Duration::from_millis(20));
    }
    // Already reaped/terminated: do not retain numeric identities for cleanup.
    callback.checker_group = None;
}
