use super::{BindingInventory, MAX_BINDING_INVENTORY, RecoveryPreview};
use crate::project::Result;
use crate::workspace::unix::{
    Storage, binding_path, expected, identity, io_error, lock, open_at, parse, private_child,
    read_receipt, required_dir, secure, selection_valid, store, validate_live,
};
use crate::workspace::{
    BindingStatus, OwnershipObservation, Receipt, WorkerMetadata, WorktreeBinding, fail, hex,
};
use std::ffi::{CStr, OsStr};
use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

fn changed() -> crate::project::Error {
    fail(
        "WORKSPACE_CHANGED",
        "binding evidence differs from the reviewed receipt",
    )
}
fn valid_work(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.ends_with(".lock")
        && !value.contains("..")
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}
struct Directory(*mut libc::DIR);
impl Drop for Directory {
    fn drop(&mut self) {
        unsafe {
            libc::closedir(self.0);
        }
    }
}
fn names(directory: &File) -> Result<Vec<String>> {
    let fd = unsafe { libc::fcntl(directory.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if fd < 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let raw = unsafe { libc::fdopendir(fd) };
    if raw.is_null() {
        unsafe {
            libc::close(fd);
        }
        return Err(io_error(std::io::Error::last_os_error()));
    }
    let directory = Directory(raw);
    let mut names = Vec::new();
    loop {
        errno::set_errno(errno::Errno(0));
        let entry = unsafe { libc::readdir(directory.0) };
        if entry.is_null() {
            if errno::errno().0 != 0 {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            break;
        }
        let raw = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) };
        if raw.to_bytes() == b"." || raw.to_bytes() == b".." {
            continue;
        }
        if names.len() > MAX_BINDING_INVENTORY {
            return Err(fail(
                "WORKSPACE_INVENTORY_LIMIT",
                "private binding inventory exceeds its entry limit",
            ));
        }
        let name = raw.to_str().map_err(|_| {
            fail(
                "WORKSPACE_INVENTORY_ENTRY",
                "private binding storage contains a non-UTF-8 entry",
            )
        })?;
        names.push(name.to_owned());
    }
    names.sort();
    Ok(names)
}
fn stamp(file: &File) -> Result<(crate::workspace::FileIdentity, i64, i64, i64, i64)> {
    let m = file.metadata().map_err(io_error)?;
    Ok((
        identity(&m),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}
pub(super) fn inventory(
    source: &Path,
    project: &str,
    managed: Option<&Path>,
) -> Result<BindingInventory> {
    let expected = expected(source, project, "project-context", "inventory", managed)?;
    let mut result = BindingInventory {
        repository_id: expected.repository_id.clone(),
        work_ids: Vec::new(),
        issues: Vec::new(),
    };
    let Some(directory) = store(&expected.common_dir, false)? else {
        return Ok(result);
    };
    let before = stamp(&directory)?;
    for name in names(&directory)? {
        if name == "creation.lock" {
            let check = open_at(&directory, OsStr::new(&name), false, false, false)
                .and_then(|file| file.ok_or_else(changed))
                .and_then(|file| secure(&file.metadata().map_err(io_error)?, false));
            if let Err(error) = check {
                result.issues.push(error);
            }
            continue;
        }
        if !valid_work(&name) {
            result.issues.push(fail(
                "WORKSPACE_INVENTORY_ENTRY",
                "private binding storage contains an invalid work ID",
            ));
            continue;
        }
        if result.work_ids.len() == MAX_BINDING_INVENTORY {
            return Err(fail(
                "WORKSPACE_INVENTORY_LIMIT",
                "private binding inventory exceeds its entry limit",
            ));
        }
        let check = private_child(&directory, &name, false).and_then(|v| v.ok_or_else(changed));
        if let Err(mut error) = check {
            error.message = format!("{name}: {}", error.message);
            result.issues.push(error);
        }
        result.work_ids.push(name);
    }
    let current = store(&expected.common_dir, false)?.ok_or_else(changed)?;
    if stamp(&directory)? != before
        || stamp(&current)? != before
        || identity(
            &required_dir(&expected.common_dir)?
                .metadata()
                .map_err(io_error)?,
        ) != expected.common_identity
    {
        return Err(changed());
    }
    Ok(result)
}

struct Snapshot {
    directory: File,
    receipt_file: File,
    receipt_stamp: (crate::workspace::FileIdentity, i64, i64, i64, i64),
    bytes: Vec<u8>,
}
impl Snapshot {
    fn read(expected: &Receipt) -> Result<Option<Self>> {
        let Some(storage) = store(&expected.common_dir, false)? else {
            return Ok(None);
        };
        let Some(directory) = private_child(&storage, &expected.work_id, false)? else {
            return Ok(None);
        };
        let receipt_file = open_at(&directory, OsStr::new("receipt.json"), false, false, false)?
            .ok_or_else(|| fail("WORKSPACE_INCOMPLETE", "binding directory has no receipt"))?;
        secure(&receipt_file.metadata().map_err(io_error)?, false)?;
        let receipt_stamp = stamp(&receipt_file)?;
        let bytes = read_receipt(&directory)?;
        let result = Self {
            directory,
            receipt_file,
            receipt_stamp,
            bytes,
        };
        result.unchanged(expected)?;
        Ok(Some(result))
    }
    fn unchanged(&self, expected: &Receipt) -> Result<()> {
        let current = required_dir(&binding_path(expected))?;
        secure(&current.metadata().map_err(io_error)?, true)?;
        let file = open_at(&current, OsStr::new("receipt.json"), false, false, false)?
            .ok_or_else(changed)?;
        secure(&file.metadata().map_err(io_error)?, false)?;
        if identity(&current.metadata().map_err(io_error)?)
            != identity(&self.directory.metadata().map_err(io_error)?)
            || stamp(&file)? != self.receipt_stamp
            || stamp(&self.receipt_file)? != self.receipt_stamp
            || read_receipt(&current)? != self.bytes
        {
            return Err(changed());
        }
        Ok(())
    }
}
fn parse_stored(bytes: &[u8], mut expected: Receipt) -> Result<Receipt> {
    let candidate: Receipt = serde_json::from_slice(bytes)
        .map_err(|_| fail("WORKSPACE_METADATA", "binding JSON or schema is invalid"))?;
    if !selection_valid(&candidate.selector) {
        return Err(fail("WORKSPACE_METADATA", "stored selector is invalid"));
    }
    expected.selector = candidate.selector;
    parse(bytes, &expected)
}
fn proven_ready(receipt: &Receipt) -> Result<Receipt> {
    if !matches!(
        receipt.status,
        BindingStatus::Preparing | BindingStatus::Failed
    ) || receipt.worker_metadata != WorkerMetadata::default()
    {
        return Err(fail(
            "WORKSPACE_RECOVERY_STATE",
            "creation recovery requires an unstaged incomplete worker",
        ));
    }
    let proof = receipt.creation_proof.as_ref().ok_or_else(|| {
        fail(
            "WORKSPACE_RECOVERY_UNPROVEN",
            "retained creation has no durable checkout identity proof",
        )
    })?;
    if proof.schema_version != 1 {
        return Err(fail(
            "WORKSPACE_RECOVERY_UNPROVEN",
            "unsupported checkout identity proof",
        ));
    }
    let mut ready = receipt.clone();
    ready.root_identity = Some(proof.root_identity);
    ready.git_dir = Some(proof.git_dir.clone());
    ready.git_identity = Some(proof.git_identity);
    ready.status = BindingStatus::Ready;
    ready.last_error = None;
    let bytes = serde_json::to_vec(&ready)
        .map_err(|_| fail("WORKSPACE_METADATA", "cannot validate recovery candidate"))?;
    let ready = parse(&bytes, &ready)?;
    validate_live(&ready)?;
    Ok(ready)
}
pub(super) fn preview(
    source: &Path,
    project: &str,
    work: &str,
    managed: Option<&Path>,
) -> Result<RecoveryPreview> {
    let expected = expected(source, project, "project-context", work, managed)?;
    let snapshot = Snapshot::read(&expected);
    let observation = WorktreeBinding::observe_with_root(source, project, work, managed)?;
    let mut result = RecoveryPreview {
        observation,
        receipt_sha256: None,
        creation_recoverable: false,
    };
    let snapshot = match snapshot {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => return Ok(result),
        Err(error) => {
            if !result
                .observation
                .issues
                .iter()
                .any(|v| v.code == error.code)
            {
                result.observation.issues.push(error);
            }
            return Ok(result);
        }
    };
    result.receipt_sha256 = Some(crate::hash(&snapshot.bytes));
    if let Err(error) = snapshot.unchanged(&expected) {
        result.observation.ownership = OwnershipObservation::Unknown;
        result.observation.issues.push(error);
        result.receipt_sha256 = None;
        return Ok(result);
    }
    if let Ok(receipt) = parse_stored(&snapshot.bytes, expected) {
        if receipt.status != BindingStatus::Ready
            && result.observation.ownership == OwnershipObservation::Available
        {
            match proven_ready(&receipt) {
                Ok(_) => result.creation_recoverable = true,
                Err(error) => result.observation.issues.push(error),
            }
        }
    }
    Ok(result)
}

fn owned_bytes(binding: &WorktreeBinding) -> Result<Vec<u8>> {
    let live = required_dir(&binding.storage.path)?;
    secure(&live.metadata().map_err(io_error)?, true)?;
    let owner =
        open_at(&live, OsStr::new("owner.lock"), false, false, false)?.ok_or_else(changed)?;
    secure(&owner.metadata().map_err(io_error)?, false)?;
    if identity(&live.metadata().map_err(io_error)?)
        != identity(&binding.storage.directory.metadata().map_err(io_error)?)
        || identity(&owner.metadata().map_err(io_error)?)
            != identity(&binding.storage.lock.metadata().map_err(io_error)?)
        || identity(
            &required_dir(&binding.receipt.common_dir)?
                .metadata()
                .map_err(io_error)?,
        ) != binding.receipt.common_identity
    {
        return Err(changed());
    }
    private_child(&live, "proxy", false)?
        .ok_or_else(|| fail("WORKSPACE_INCOMPLETE", "private proxy directory is missing"))?;
    let bytes = read_receipt(&live)?;
    if Some(crate::hash(&bytes).as_str()) != binding.storage.expected_hash.as_deref() {
        return Err(changed());
    }
    Ok(bytes)
}
mod archive;
use archive::archive;

pub(super) fn update_metadata(
    binding: &mut WorktreeBinding,
    metadata: WorkerMetadata,
    expected_hash: &str,
) -> Result<()> {
    if !hex(expected_hash, 64) || binding.receipt_digest()? != expected_hash {
        return Err(changed());
    }
    metadata.validate()?;
    validate_live(&binding.receipt)?;
    let before = owned_bytes(binding)?;
    archive(binding, &before)?;
    let mut next = binding.receipt.clone();
    next.worker_metadata = metadata;
    binding.storage.write(&next)?;
    binding.receipt = next;
    Ok(())
}
pub(super) fn recover_creation(
    source: &Path,
    project: &str,
    work: &str,
    expected_hash: &str,
    plan_sha256: Option<&str>,
    managed: Option<&Path>,
) -> Result<WorktreeBinding> {
    if !hex(expected_hash, 64) || plan_sha256.is_some_and(|hash| !hex(hash, 64)) {
        return Err(fail(
            "WORKSPACE_RECOVERY_HASH",
            "recovery requires exact SHA-256 evidence",
        ));
    }
    let expected = expected(source, project, "project-context", work, managed)?;
    let store = store(&expected.common_dir, false)?
        .ok_or_else(|| fail("WORKSPACE_NOT_BOUND", "no private binding exists"))?;
    let directory = private_child(&store, work, false)?
        .ok_or_else(|| fail("WORKSPACE_NOT_BOUND", "no private binding exists"))?;
    let owner = lock(&directory, "owner.lock", false, false)?;
    let bytes = read_receipt(&directory)?;
    if crate::hash(&bytes) != expected_hash {
        return Err(changed());
    }
    let receipt = parse_stored(&bytes, expected)?;
    let mut ready = proven_ready(&receipt)?;
    let path = binding_path(&receipt);
    let mut binding = WorktreeBinding {
        receipt,
        proxy_path: path.join("proxy"),
        newly_created: false,
        storage: Storage {
            directory,
            lock: owner,
            path,
            expected_hash: Some(expected_hash.into()),
        },
    };
    let before = owned_bytes(&binding)?;
    archive(&binding, &before)?;
    ready.worker_metadata.last_recovery_sha256 = plan_sha256.map(str::to_owned);
    binding.storage.write(&ready)?;
    binding.receipt = ready;
    Ok(binding)
}
