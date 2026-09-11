//! Private destination receipts and advisory ownership for native launches.
//!
//! These files contain local routing metadata, never portable context or authority.
//! Locks exclude cooperating Astral owners; the current OS user remains trusted.

use crate::project::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::path::{Path, PathBuf};

mod observation;
pub use observation::{
    LaunchContinuation, LaunchInventory, LaunchObservation, LaunchOwnership, LaunchReceiptSummary,
    MAX_INVENTORY_ENTRIES, MAX_INVENTORY_PAGE,
};

pub const MAX_RECEIPT_BYTES: usize = 16_384;
#[cfg(unix)]
const MAX_ATTEMPTS: usize = 16;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Status {
    Preparing,
    Staged,
    Finished,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    id: String,
    workspace: String,
    project_id: String,
    selector: String,
    #[serde(deserialize_with = "required_option")]
    work_id: Option<String>,
    selection_digest: String,
    bundle_manifest_sha256: String,
    #[serde(deserialize_with = "required_option")]
    model: Option<String>,
    #[serde(deserialize_with = "required_option")]
    provider: Option<String>,
    #[serde(deserialize_with = "required_option")]
    thread_id: Option<String>,
    status: Status,
    #[serde(deserialize_with = "required_option")]
    exit_code: Option<i32>,
    #[serde(deserialize_with = "required_option")]
    last_error: Option<String>,
}

fn required_option<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

/// The returned object owns its exclusive launch lock until dropped.
pub struct LaunchState {
    receipt: Receipt,
    proxy_path: PathBuf,
    expected_hash: Option<String>,
    #[cfg(unix)]
    directory: std::fs::File,
    #[cfg(unix)]
    _lock: unix::Ownership,
}

fn fail(code: &'static str, message: &'static str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn hex(value: &str, bytes: usize) -> bool {
    value.len() == bytes
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn logical(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn selector_valid(value: &str) -> bool {
    if let Some((kind, id)) = value.split_once(':') {
        matches!(kind, "subsystem" | "projection") && logical(id)
    } else {
        logical(value)
    }
}

fn error_code(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

fn runtime_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
}

fn thread_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
            }
        })
        && value.bytes().any(|b| b != b'0' && b != b'-')
}

fn identity(
    root: &Path,
    project_id: &str,
    selector: &str,
    work_id: Option<&str>,
    selection_digest: &str,
    bundle_sha: &str,
) -> Result<Receipt> {
    if !logical(project_id)
        || !selector_valid(selector)
        || work_id.is_some_and(|v| !logical(v))
        || !hex(selection_digest, 64)
        || !hex(bundle_sha, 64)
    {
        return Err(fail(
            "LAUNCH_STATE_IDENTITY",
            "invalid launch identity metadata",
        ));
    }
    let root = root
        .canonicalize()
        .map_err(|_| fail("LAUNCH_STATE_WORKSPACE", "workspace cannot be resolved"))?;
    if !root.is_dir() {
        return Err(fail(
            "LAUNCH_STATE_WORKSPACE",
            "workspace must be a directory",
        ));
    }
    let workspace = root
        .to_str()
        .filter(|v| v.len() <= 4096)
        .ok_or_else(|| {
            fail(
                "LAUNCH_STATE_WORKSPACE",
                "receipt workspace requires a bounded UTF-8 path",
            )
        })?
        .to_owned();
    Ok(Receipt {
        schema_version: 1,
        id: String::new(),
        workspace,
        project_id: project_id.into(),
        selector: selector.into(),
        work_id: work_id.map(str::to_owned),
        selection_digest: selection_digest.into(),
        bundle_manifest_sha256: bundle_sha.into(),
        model: None,
        provider: None,
        thread_id: None,
        status: Status::Preparing,
        exit_code: None,
        last_error: None,
    })
}

fn configured_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("ASTRAL_LAUNCH_STATE_DIR") {
        if path.is_empty() {
            return Err(fail(
                "LAUNCH_STATE_PATH",
                "launch state directory must be absolute",
            ));
        }
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            fail(
                "LAUNCH_STATE_PATH",
                "HOME or ASTRAL_LAUNCH_STATE_DIR is required",
            )
        })?;
    Ok(PathBuf::from(home).join(".local/state/astral/launches"))
}

#[cfg(unix)]
fn validate_receipt(receipt: &Receipt, id: &str, expected: &Receipt) -> Result<()> {
    if receipt.schema_version != 1
        || receipt.id != id
        || !hex(&receipt.id, 32)
        || receipt.workspace != expected.workspace
        || receipt.project_id != expected.project_id
        || receipt.selector != expected.selector
        || receipt.work_id != expected.work_id
        || receipt.selection_digest != expected.selection_digest
        || receipt.bundle_manifest_sha256 != expected.bundle_manifest_sha256
    {
        return Err(fail(
            "LAUNCH_STATE_MISMATCH",
            "launch receipt differs from the selected workspace or context",
        ));
    }
    validate_receipt_metadata(receipt)
}

#[cfg(unix)]
fn validate_receipt_metadata(receipt: &Receipt) -> Result<()> {
    if receipt.schema_version != 1
        || !hex(&receipt.id, 32)
        || receipt.workspace.len() > 4096
        || !Path::new(&receipt.workspace).is_absolute()
        || receipt.workspace.contains('\0')
        || !logical(&receipt.project_id)
        || !selector_valid(&receipt.selector)
        || receipt.work_id.as_deref().is_some_and(|v| !logical(v))
        || !hex(&receipt.selection_digest, 64)
        || !hex(&receipt.bundle_manifest_sha256, 64)
    {
        return Err(fail(
            "LAUNCH_STATE_INVALID",
            "launch receipt contains invalid identity metadata",
        ));
    }
    let staged = match (&receipt.thread_id, &receipt.model, &receipt.provider) {
        (Some(thread), Some(model), Some(provider)) => {
            if !thread_uuid(thread) || !runtime_name(model) || !runtime_name(provider) {
                return Err(fail(
                    "LAUNCH_STATE_INVALID",
                    "launch receipt contains invalid runtime metadata",
                ));
            }
            true
        }
        (None, None, None) => false,
        _ => {
            return Err(fail(
                "LAUNCH_STATE_INVALID",
                "launch receipt contains incomplete runtime metadata",
            ));
        }
    };
    let boundary_valid = match receipt.status {
        Status::Preparing => !staged,
        Status::Staged | Status::Finished => staged,
        Status::Failed => true,
    };
    if !boundary_valid
        || receipt.status != Status::Finished && receipt.exit_code.is_some()
        || (receipt.status == Status::Failed) != receipt.last_error.is_some()
        || receipt
            .last_error
            .as_deref()
            .is_some_and(|value| !error_code(value))
    {
        return Err(fail(
            "LAUNCH_STATE_INVALID",
            "launch receipt contains invalid lifecycle metadata",
        ));
    }
    Ok(())
}

impl LaunchState {
    pub fn create(
        root: &Path,
        project_id: &str,
        selector: &str,
        work_id: Option<&str>,
        selection_digest: &str,
        bundle_sha: &str,
    ) -> Result<Self> {
        Self::create_in(
            &configured_directory()?,
            root,
            project_id,
            selector,
            work_id,
            selection_digest,
            bundle_sha,
        )
    }

    /// Explicit trusted host state directory, useful for embeddings and isolated tests.
    /// Never resolve this directory from portable project metadata.
    pub fn create_in(
        state_dir: &Path,
        root: &Path,
        project_id: &str,
        selector: &str,
        work_id: Option<&str>,
        selection_digest: &str,
        bundle_sha: &str,
    ) -> Result<Self> {
        let receipt = identity(
            root,
            project_id,
            selector,
            work_id,
            selection_digest,
            bundle_sha,
        )?;
        #[cfg(unix)]
        {
            unix::create(state_dir, receipt)
        }
        #[cfg(not(unix))]
        {
            let _ = (state_dir, receipt);
            Err(fail(
                "LAUNCH_STATE_UNSUPPORTED_PLATFORM",
                "private launch state currently requires Unix",
            ))
        }
    }

    pub fn resume(
        id: &str,
        root: &Path,
        project_id: &str,
        selector: &str,
        work_id: Option<&str>,
        selection_digest: &str,
        bundle_sha: &str,
    ) -> Result<Self> {
        Self::resume_in(
            &configured_directory()?,
            id,
            root,
            project_id,
            selector,
            work_id,
            selection_digest,
            bundle_sha,
        )
    }

    #[allow(clippy::too_many_arguments)] // The explicit directory supplements the same immutable identity as resume.
    pub fn resume_in(
        state_dir: &Path,
        id: &str,
        root: &Path,
        project_id: &str,
        selector: &str,
        work_id: Option<&str>,
        selection_digest: &str,
        bundle_sha: &str,
    ) -> Result<Self> {
        if !hex(id, 32) {
            return Err(fail(
                "LAUNCH_STATE_ID",
                "launch ID must contain 32 lowercase hexadecimal characters",
            ));
        }
        let expected = identity(
            root,
            project_id,
            selector,
            work_id,
            selection_digest,
            bundle_sha,
        )?;
        #[cfg(unix)]
        {
            unix::resume(state_dir, id, expected)
        }
        #[cfg(not(unix))]
        {
            let _ = (state_dir, expected);
            Err(fail(
                "LAUNCH_STATE_UNSUPPORTED_PLATFORM",
                "private launch state currently requires Unix",
            ))
        }
    }

    pub fn id(&self) -> &str {
        &self.receipt.id
    }
    pub fn selector(&self) -> &str {
        &self.receipt.selector
    }
    pub fn work_id(&self) -> Option<&str> {
        self.receipt.work_id.as_deref()
    }
    pub fn selection_digest(&self) -> &str {
        &self.receipt.selection_digest
    }
    pub fn bundle_manifest_sha256(&self) -> &str {
        &self.receipt.bundle_manifest_sha256
    }
    pub fn receipt_sha256(&self) -> Option<&str> {
        self.expected_hash.as_deref()
    }
    pub fn proxy_state_dir(&self) -> &Path {
        &self.proxy_path
    }
    pub fn thread_id(&self) -> Option<&str> {
        self.receipt.thread_id.as_deref()
    }
    pub fn model(&self) -> Option<&str> {
        self.receipt.model.as_deref()
    }
    pub fn provider(&self) -> Option<&str> {
        self.receipt.provider.as_deref()
    }

    pub fn status(&self) -> &'static str {
        match self.receipt.status {
            Status::Preparing => "preparing",
            Status::Staged => "staged",
            Status::Finished => "finished",
            Status::Failed => "failed",
        }
    }

    pub fn staged(&mut self, thread: &str, model: &str, provider: &str) -> Result<()> {
        if self.receipt.thread_id.is_some() {
            return Err(fail(
                "LAUNCH_ALREADY_STAGED",
                "launch already has a staged thread; resume does not inject context again",
            ));
        }
        if !thread_uuid(thread) || !runtime_name(model) || !runtime_name(provider) {
            return Err(fail(
                "LAUNCH_STATE_INVALID",
                "invalid staged thread or runtime identity",
            ));
        }
        let mut next = self.receipt.clone();
        next.thread_id = Some(thread.into());
        next.model = Some(model.into());
        next.provider = Some(provider.into());
        next.status = Status::Staged;
        next.exit_code = None;
        next.last_error = None;
        self.update(next)
    }

    /// Records the child's terminal exit, including nonzero or signal termination.
    pub fn finished(&mut self, exit: Option<i32>) -> Result<()> {
        if self.receipt.thread_id.is_none() {
            return Err(fail("LAUNCH_NOT_STAGED", "launch has no staged thread"));
        }
        let mut next = self.receipt.clone();
        next.status = Status::Finished;
        next.exit_code = exit;
        next.last_error = None;
        self.update(next)
    }

    /// Store a bounded error code only, never an exception body or stderr.
    pub fn failed(&mut self, code: &str) -> Result<()> {
        let mut next = self.receipt.clone();
        next.status = Status::Failed;
        next.exit_code = None;
        next.last_error = Some(
            if error_code(code) {
                code
            } else {
                "LAUNCH_FAILED"
            }
            .into(),
        );
        self.update(next)
    }

    fn update(&mut self, next: Receipt) -> Result<()> {
        #[cfg(unix)]
        {
            let hash = unix::write_receipt(&self.directory, &next, self.expected_hash.as_deref())?;
            self.receipt = next;
            self.expected_hash = Some(hash);
            Ok(())
        }
        #[cfg(not(unix))]
        {
            let _ = next;
            Err(fail(
                "LAUNCH_STATE_UNSUPPORTED_PLATFORM",
                "private launch state currently requires Unix",
            ))
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::ffi::{CString, OsStr};
    use std::fs::{File, Metadata};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Component;

    fn name(value: &OsStr) -> Result<CString> {
        CString::new(value.as_bytes())
            .map_err(|_| fail("LAUNCH_STATE_PATH", "launch state path contains a NUL byte"))
    }

    fn uid() -> u32 {
        unsafe { libc::geteuid() }
    }

    pub(super) fn secure(metadata: &Metadata, directory: bool) -> Result<()> {
        if metadata.uid() != uid()
            || metadata.mode() & 0o7777 != if directory { 0o700 } else { 0o600 }
            || if directory {
                !metadata.is_dir()
            } else {
                !metadata.is_file() || metadata.nlink() != 1
            }
        {
            return Err(fail(
                "LAUNCH_STATE_INSECURE",
                "launch state must be private, owned by the current user, and contain only expected file types",
            ));
        }
        Ok(())
    }

    fn open_at(
        parent: &File,
        component: &OsStr,
        directory: bool,
        create_file: bool,
    ) -> Result<File> {
        let component = name(component)?;
        let flags = libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory {
                libc::O_RDONLY | libc::O_DIRECTORY
            } else {
                libc::O_RDWR
            }
            | if create_file {
                libc::O_CREAT | libc::O_EXCL
            } else {
                0
            };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), component.as_ptr(), flags, 0o600) };
        if fd < 0 {
            return Err(fail(
                "LAUNCH_STATE_UNAVAILABLE",
                "launch state entry is missing, unsafe, or unavailable",
            ));
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn mkdir_at(parent: &File, component: &OsStr) -> std::io::Result<bool> {
        let component = CString::new(component.as_bytes())?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), component.as_ptr(), 0o700) } == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Ok(false)
        } else {
            Err(error)
        }
    }

    pub(super) fn directory_at(parent: &File, component: &OsStr, create: bool) -> Result<File> {
        if create {
            mkdir_at(parent, component).map_err(|_| {
                fail(
                    "LAUNCH_STATE_UNAVAILABLE",
                    "cannot create private launch directory",
                )
            })?;
        }
        let file = open_at(parent, component, true, false)?;
        secure(
            &file
                .metadata()
                .map_err(|_| fail("LAUNCH_STATE_IO", "cannot inspect launch directory"))?,
            true,
        )?;
        Ok(file)
    }

    pub(super) fn state_root(path: &Path, workspace: &str, create: bool) -> Result<File> {
        if !path.is_absolute() || path.as_os_str().as_bytes().len() > 4096 {
            return Err(fail(
                "LAUNCH_STATE_PATH",
                "launch state directory must be a bounded absolute path",
            ));
        }
        // Component validation also prevents ambiguous dot/parent aliases.
        if path
            .as_os_str()
            .as_bytes()
            .split(|b| *b == b'/')
            .skip(1)
            .any(|part| part.is_empty() || part == b"." || part == b"..")
        {
            return Err(fail(
                "LAUNCH_STATE_PATH",
                "launch state directory must have unambiguous path components",
            ));
        }
        if path.starts_with(workspace) {
            return Err(fail(
                "LAUNCH_STATE_IN_WORKSPACE",
                "destination launch receipts must be outside the project workspace",
            ));
        }
        let mut directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")
            .map_err(|_| fail("LAUNCH_STATE_UNAVAILABLE", "cannot open filesystem root"))?;
        let components: Vec<_> = path
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        if components.is_empty() {
            return Err(fail(
                "LAUNCH_STATE_PATH",
                "filesystem root is not a private launch directory",
            ));
        }
        for (index, component) in components.iter().enumerate() {
            if create {
                mkdir_at(&directory, component).map_err(|_| {
                    fail(
                        "LAUNCH_STATE_UNAVAILABLE",
                        "cannot create launch state parent",
                    )
                })?;
            }
            directory = open_at(&directory, component, true, false)?;
            let metadata = directory
                .metadata()
                .map_err(|_| fail("LAUNCH_STATE_IO", "cannot inspect launch state parent"))?;
            if index + 1 == components.len() {
                secure(&metadata, true)?;
            } else if !metadata.is_dir()
                || ![0, uid()].contains(&metadata.uid())
                || (metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0)
            {
                return Err(fail(
                    "LAUNCH_STATE_INSECURE",
                    "launch state ancestor permits unsafe replacement",
                ));
            }
        }
        Ok(directory)
    }

    fn random_id() -> Result<String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|_| fail("LAUNCH_STATE_ENTROPY", "OS randomness is unavailable"))?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    }

    pub(super) struct Ownership(File);

    impl Drop for Ownership {
        fn drop(&mut self) {
            // Forked children can retain a CLOEXEC descriptor until exec. End
            // ownership now, rather than waiting for the final descriptor close.
            let _ = fs2::FileExt::unlock(&self.0);
        }
    }

    fn lock(directory: &File, create: bool) -> Result<Ownership> {
        let file = open_at(directory, OsStr::new("owner.lock"), false, create)?;
        secure(
            &file
                .metadata()
                .map_err(|_| fail("LAUNCH_STATE_IO", "cannot inspect launch lock"))?,
            false,
        )?;
        fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                fail("LAUNCH_ACTIVE", "another process owns this launch")
            } else {
                fail("LAUNCH_STATE_LOCK", "cannot acquire launch ownership")
            }
        })?;
        Ok(Ownership(file))
    }

    pub(super) fn create(path: &Path, mut receipt: Receipt) -> Result<LaunchState> {
        let parent = state_root(path, &receipt.workspace, true)?;
        for _ in 0..MAX_ATTEMPTS {
            let id = random_id()?;
            if !mkdir_at(&parent, OsStr::new(&id))
                .map_err(|_| fail("LAUNCH_STATE_IO", "cannot reserve launch ID"))?
            {
                continue;
            }
            let directory = directory_at(&parent, OsStr::new(&id), false)?;
            let lock = lock(&directory, true)?;
            directory_at(&directory, OsStr::new("proxy"), true)?;
            receipt.id = id.clone();
            let mut state = LaunchState {
                receipt: receipt.clone(),
                proxy_path: path.join(id).join("proxy"),
                expected_hash: None,
                directory,
                _lock: lock,
            };
            state.update(receipt)?;
            parent
                .sync_all()
                .map_err(|_| fail("LAUNCH_STATE_IO", "cannot persist launch directory"))?;
            return Ok(state);
        }
        Err(fail(
            "LAUNCH_STATE_COLLISIONS",
            "launch ID reservation attempts exhausted",
        ))
    }

    pub(super) fn resume(path: &Path, id: &str, expected: Receipt) -> Result<LaunchState> {
        let parent = state_root(path, &expected.workspace, false)?;
        let directory = directory_at(&parent, OsStr::new(id), false)?;
        let lock = lock(&directory, false)?;
        let bytes = read_receipt(&directory)?;
        let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| {
            fail(
                "LAUNCH_STATE_INVALID",
                "launch receipt JSON or schema is invalid",
            )
        })?;
        validate_receipt(&receipt, id, &expected)?;
        if receipt.thread_id.is_none() {
            return Err(fail(
                "LAUNCH_NOT_STAGED",
                "launch has no staged thread; failure receipt was retained",
            ));
        }
        directory_at(&directory, OsStr::new("proxy"), false)?;
        Ok(LaunchState {
            receipt,
            proxy_path: path.join(id).join("proxy"),
            expected_hash: Some(crate::hash(&bytes)),
            directory,
            _lock: lock,
        })
    }

    fn read_receipt(directory: &File) -> Result<Vec<u8>> {
        let file = open_at(directory, OsStr::new("receipt.json"), false, false)?;
        let metadata = file
            .metadata()
            .map_err(|_| fail("LAUNCH_STATE_IO", "cannot inspect launch receipt"))?;
        secure(&metadata, false)?;
        if metadata.len() > MAX_RECEIPT_BYTES as u64 {
            return Err(fail(
                "LAUNCH_STATE_LIMIT",
                "launch receipt exceeds byte limit",
            ));
        }
        let mut bytes = Vec::new();
        file.take(MAX_RECEIPT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| fail("LAUNCH_STATE_IO", "cannot read launch receipt"))?;
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(fail(
                "LAUNCH_STATE_LIMIT",
                "launch receipt exceeds byte limit",
            ));
        }
        Ok(bytes)
    }

    pub(super) fn write_receipt(
        directory: &File,
        receipt: &Receipt,
        expected_hash: Option<&str>,
    ) -> Result<String> {
        if let Some(expected_hash) = expected_hash {
            if crate::hash(&read_receipt(directory)?) != expected_hash {
                return Err(fail(
                    "LAUNCH_STATE_CHANGED",
                    "launch receipt changed while owned",
                ));
            }
        }
        let bytes = serde_json::to_vec(receipt)
            .map_err(|_| fail("LAUNCH_STATE_INVALID", "cannot encode launch receipt"))?;
        if bytes.len() > MAX_RECEIPT_BYTES {
            return Err(fail(
                "LAUNCH_STATE_LIMIT",
                "launch receipt exceeds byte limit",
            ));
        }
        let temp_name = format!(".receipt-{}.tmp", random_id()?);
        let temp = name(OsStr::new(&temp_name))?;
        let target = name(OsStr::new("receipt.json"))?;
        let mut file = open_at(directory, OsStr::new(&temp_name), false, true)?;
        let result = (|| {
            secure(
                &file
                    .metadata()
                    .map_err(|_| fail("LAUNCH_STATE_IO", "cannot inspect temporary receipt"))?,
                false,
            )?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| fail("LAUNCH_STATE_IO", "cannot persist launch receipt"))?;
            if unsafe {
                libc::renameat(
                    directory.as_raw_fd(),
                    temp.as_ptr(),
                    directory.as_raw_fd(),
                    target.as_ptr(),
                )
            } != 0
            {
                return Err(fail("LAUNCH_STATE_IO", "cannot replace launch receipt"));
            }
            directory
                .sync_all()
                .map_err(|_| fail("LAUNCH_STATE_IO", "cannot persist receipt directory"))?;
            Ok(crate::hash(&bytes))
        })();
        if result.is_err() {
            unsafe {
                libc::unlinkat(directory.as_raw_fd(), temp.as_ptr(), 0);
            }
        }
        result
    }

    #[test]
    fn release_does_not_leave_ownership_in_an_inherited_descriptor() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("workspace");
        std::fs::create_dir(&root).unwrap();
        let store = temp.path().canonicalize().unwrap().join("state");
        let mut state = LaunchState::create_in(
            &store,
            &root,
            "fixture",
            "projection:saved",
            None,
            &"a".repeat(64),
            &"b".repeat(64),
        )
        .unwrap();
        state
            .staged(
                "01a10000-1234-7000-8000-000000000001",
                "gpt-6-astra",
                "openai",
            )
            .unwrap();
        let id = state.id().to_owned();
        // A duplicate shares the open-file description, like a forked child
        // before exec closes its CLOEXEC descriptors.
        let inherited = state._lock.0.try_clone().unwrap();
        drop(state);
        let resumed = LaunchState::resume_in(
            &store,
            &id,
            &root,
            "fixture",
            "projection:saved",
            None,
            &"a".repeat(64),
            &"b".repeat(64),
        )
        .unwrap();
        drop(inherited);
        drop(resumed);
    }
}
