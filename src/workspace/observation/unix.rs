use super::{BindingObservation, OwnershipObservation};
use crate::project::{Error, Result};
use crate::workspace::unix::{
    collision, expected, identity, io_error, open_at, parse, private_child, required_dir, secure,
    selection_valid, store, validate_live,
};
use crate::workspace::{BindingStatus, FileIdentity, MAX_BINDING_BYTES, Receipt, fail};
use std::ffi::OsStr;
use std::fs::{File, Metadata};
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn changed() -> Error {
    fail(
        "WORKSPACE_CHANGED",
        "binding evidence changed during observation; observe again before acting",
    )
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    identity: FileIdentity,
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl FileStamp {
    fn of(metadata: &Metadata) -> Self {
        Self {
            identity: identity(metadata),
            bytes: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}

/// Unlike an ordinary receipt read, retain the opened inode as well as the bytes
/// so an atomic replacement with identical content still invalidates this view.
struct ReceiptSnapshot {
    _file: File,
    stamp: FileStamp,
    bytes: Vec<u8>,
}

impl ReceiptSnapshot {
    fn read(directory: &File) -> Result<Self> {
        let mut file = open_at(directory, OsStr::new("receipt.json"), false, false, false)?
            .ok_or_else(|| fail("WORKSPACE_INCOMPLETE", "binding directory has no receipt"))?;
        let metadata = file.metadata().map_err(io_error)?;
        secure(&metadata, false)?;
        if metadata.len() > MAX_BINDING_BYTES as u64 {
            return Err(fail(
                "WORKSPACE_METADATA_LIMIT",
                "binding receipt exceeds its size limit",
            ));
        }
        let stamp = FileStamp::of(&metadata);
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_BINDING_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > MAX_BINDING_BYTES {
            return Err(fail(
                "WORKSPACE_METADATA_LIMIT",
                "binding receipt exceeds its size limit",
            ));
        }
        let after = file.metadata().map_err(io_error)?;
        secure(&after, false)?;
        if stamp != FileStamp::of(&after) || metadata.len() != bytes.len() as u64 {
            return Err(changed());
        }
        Ok(Self {
            _file: file,
            stamp,
            bytes,
        })
    }

    fn unchanged(&self, directory: &File) -> Result<()> {
        let current = Self::read(directory)?;
        if self.stamp != current.stamp || self.bytes != current.bytes {
            return Err(changed());
        }
        Ok(())
    }
}

struct Probe<'a> {
    file: &'a File,
    acquired: bool,
}

impl<'a> Probe<'a> {
    fn start(file: &'a File) -> Result<Self> {
        match fs2::FileExt::try_lock_exclusive(file) {
            Ok(()) => Ok(Self {
                file,
                acquired: true,
            }),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(Self {
                file,
                acquired: false,
            }),
            Err(_) => Err(fail("WORKSPACE_LOCK", "cannot probe workspace ownership")),
        }
    }

    fn release(mut self) -> Result<OwnershipObservation> {
        if !self.acquired {
            return Ok(OwnershipObservation::Busy);
        }
        fs2::FileExt::unlock(self.file)
            .map_err(|_| fail("WORKSPACE_LOCK", "cannot release workspace ownership probe"))?;
        self.acquired = false;
        Ok(OwnershipObservation::Available)
    }
}

impl Drop for Probe<'_> {
    fn drop(&mut self) {
        if self.acquired {
            let _ = fs2::FileExt::unlock(self.file);
        }
    }
}

fn owner(directory: &File) -> Result<File> {
    let file = open_at(directory, OsStr::new("owner.lock"), false, false, false)?
        .ok_or_else(|| fail("WORKSPACE_INCOMPLETE", "binding ownership lock is missing"))?;
    secure(&file.metadata().map_err(io_error)?, false)?;
    Ok(file)
}

fn same_directory(first: &File, current: &File) -> Result<()> {
    if identity(&first.metadata().map_err(io_error)?)
        != identity(&current.metadata().map_err(io_error)?)
    {
        return Err(changed());
    }
    Ok(())
}

fn inspect_binding(mut expected: Receipt, observation: &mut BindingObservation) -> Result<()> {
    let Some(storage) = store(&expected.common_dir, false)? else {
        return collision(&expected);
    };
    let Some(directory) = private_child(&storage, &expected.work_id, false)? else {
        return collision(&expected);
    };
    let snapshot = ReceiptSnapshot::read(&directory)?;
    // Read the stored selector as inert data. Every path still comes from the
    // trusted caller and Git discovery, then the complete receipt must match it.
    let candidate: Receipt = serde_json::from_slice(&snapshot.bytes)
        .map_err(|_| fail("WORKSPACE_METADATA", "binding JSON or schema is invalid"))?;
    if !selection_valid(&candidate.selector) {
        return Err(fail(
            "WORKSPACE_METADATA",
            "stored binding selector is invalid",
        ));
    }
    expected.selector = candidate.selector;
    let receipt = parse(&snapshot.bytes, &expected)?;
    observation.selector = Some(receipt.selector.clone());
    observation.plan = Some(receipt.plan(true));

    let ownership_file = owner(&directory)?;
    let ownership_identity = identity(&ownership_file.metadata().map_err(io_error)?);
    let proxy_directory = private_child(&directory, "proxy", false)?;
    if receipt.status == BindingStatus::Ready {
        if proxy_directory.is_none() {
            return Err(fail(
                "WORKSPACE_INCOMPLETE",
                "private proxy directory is missing",
            ));
        }
        validate_live(&receipt)?;
    }

    // Keep the ownership probe brief: all subprocess validation happens before
    // it. Reopen the named evidence while probing to catch swaps or publication.
    let probe = Probe::start(&ownership_file)?;
    let common = required_dir(&expected.common_dir)?;
    if identity(&common.metadata().map_err(io_error)?) != expected.common_identity {
        return Err(changed());
    }
    let current_storage = store(&expected.common_dir, false)?.ok_or_else(changed)?;
    same_directory(&storage, &current_storage)?;
    let current_directory =
        private_child(&current_storage, &expected.work_id, false)?.ok_or_else(changed)?;
    same_directory(&directory, &current_directory)?;
    let current_owner = owner(&current_directory)?;
    if identity(&current_owner.metadata().map_err(io_error)?) != ownership_identity {
        return Err(changed());
    }
    match (
        proxy_directory,
        private_child(&current_directory, "proxy", false)?,
    ) {
        (Some(before), Some(after)) => same_directory(&before, &after)?,
        (None, None) => (),
        _ => return Err(changed()),
    }
    if receipt.status == BindingStatus::Ready {
        for (path, expected_identity) in [
            (&receipt.root, receipt.root_identity),
            (
                receipt
                    .git_dir
                    .as_ref()
                    .expect("validated ready Git directory"),
                receipt.git_identity,
            ),
        ] {
            if Some(identity(&required_dir(path)?.metadata().map_err(io_error)?))
                != expected_identity
            {
                return Err(changed());
            }
        }
    }
    snapshot.unchanged(&current_directory)?;
    observation.ownership = probe.release()?;
    if receipt.status != BindingStatus::Ready
        && observation.ownership == OwnershipObservation::Available
    {
        observation.issues.push(fail(
            "WORKSPACE_INCOMPLETE",
            "worktree creation is incomplete and no owner was observed; retained binding requires review",
        ));
    }
    Ok(())
}

pub(super) fn observe(
    source: &Path,
    project: &str,
    work: &str,
    managed: Option<&Path>,
) -> Result<BindingObservation> {
    // Caller and repository errors remain errors even when no binding exists.
    // This placeholder only locates storage; it never overrides a stored selector.
    let expected = expected(source, project, "project-context", work, managed)?;
    let mut observation = BindingObservation {
        work_id: work.into(),
        selector: None,
        plan: None,
        ownership: OwnershipObservation::Unknown,
        issues: Vec::new(),
    };
    if let Err(error) = inspect_binding(expected, &mut observation) {
        observation.ownership = OwnershipObservation::Unknown;
        observation.issues.push(error);
    }
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::OpenOptionsExt;

    #[test]
    fn receipt_snapshot_detects_atomic_replacement_with_identical_bytes() {
        let root = tempfile::tempdir().unwrap();
        let directory = File::open(root.path()).unwrap();
        let write = |name: &str| {
            use std::io::Write;
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(root.path().join(name))
                .unwrap();
            file.write_all(b"same bytes").unwrap();
        };
        write("receipt.json");
        let before = ReceiptSnapshot::read(&directory).unwrap();
        before.unchanged(&directory).unwrap();
        write("replacement.json");
        fs::rename(
            root.path().join("replacement.json"),
            root.path().join("receipt.json"),
        )
        .unwrap();
        assert_eq!(
            before.unchanged(&directory).unwrap_err().code,
            "WORKSPACE_CHANGED"
        );
    }
}
