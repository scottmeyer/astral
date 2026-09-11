use super::{WorktreeBinding, fail, io_error, open_at, private_child, secure};
use crate::project::Result;
use std::ffi::{CString, OsStr};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;

fn publish(directory: &File, pending: &str, destination: &str) -> Result<()> {
    let pending = CString::new(pending).expect("generated archive filename");
    let destination = CString::new(destination).expect("generated archive filename");
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            directory.as_raw_fd(),
            pending.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            directory.as_raw_fd(),
            pending.as_ptr(),
            directory.as_raw_fd(),
            destination.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let result = {
        let _ = (directory, pending, destination);
        return Err(fail(
            "WORKSPACE_RECOVERY_UNSUPPORTED",
            "atomic recovery evidence publication requires Linux or macOS",
        ));
    };
    if result != 0 {
        return Err(fail(
            "WORKSPACE_RECOVERY_EVIDENCE",
            "archive destination appeared or atomic publication failed; pending evidence was retained",
        ));
    }
    Ok(())
}

pub(super) fn archive(binding: &WorktreeBinding, bytes: &[u8]) -> Result<()> {
    let directory = private_child(&binding.storage.directory, "recovery-evidence", true)?
        .expect("created evidence directory");
    let digest = crate::hash(bytes);
    let name = format!("receipt-{digest}.json");
    if let Some(file) = open_at(&directory, OsStr::new(&name), false, false, false)? {
        secure(&file.metadata().map_err(io_error)?, false)?;
        let mut retained = Vec::new();
        file.take(crate::workspace::MAX_BINDING_BYTES as u64 + 1)
            .read_to_end(&mut retained)
            .map_err(io_error)?;
        if retained != bytes {
            return Err(fail(
                "WORKSPACE_RECOVERY_EVIDENCE",
                "existing receipt evidence differs from its content address",
            ));
        }
    } else {
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| fail("WORKSPACE_ENTROPY", "OS randomness is unavailable"))?;
        let pending = format!("pending-{digest}-{}.json", crate::hash(&random));
        let mut file = open_at(&directory, OsStr::new(&pending), false, true, true)?
            .expect("created pending evidence file");
        file.write_all(bytes).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        // Failed or interrupted writes retain their unique name and do not block
        // a later attempt from publishing the complete content-addressed receipt.
        publish(&directory, &pending, &name)?;
    }
    directory.sync_all().map_err(io_error)?;
    binding.storage.directory.sync_all().map_err(io_error)?;
    Ok(())
}
