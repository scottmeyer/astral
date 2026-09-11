//! Unix Git worktree bindings. Receipts are private metadata, never runtime options.
//!
//! Ownership is advisory: cooperating Astral processes exclude one another, while
//! the current OS user and Git executable remain trusted. Inspection is an
//! unlocked observation, not a reservation. No failed creation is automatically
//! repaired, adopted, pruned, or deleted.

use crate::project::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_BINDING_BYTES: usize = 16_384;

mod observation;
pub use observation::{BindingObservation, OwnershipObservation};
mod recovery;
pub use recovery::{BindingInventory, MAX_BINDING_INVENTORY, RecoveryPreview};

/// Shared bounded Git invocation; current process supplies every literal argument.
pub(crate) fn git_read(root: &Path, args: &[std::ffi::OsString]) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        unix::git(
            root,
            &args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>(),
            false,
        )
        .map(|(_, bytes)| bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, args);
        Err(unsupported())
    }
}

/// A bounded larger read for committed native blobs; normal Git plumbing keeps
/// its existing one-MiB output cap. This never changes the subprocess deadline.
pub(crate) fn git_read_with_limit(
    root: &Path,
    args: &[std::ffi::OsString],
    limit: usize,
) -> Result<Vec<u8>> {
    if limit > crate::native_bundle::MAX_PAYLOAD_BYTES {
        return Err(fail(
            "WORKSPACE_GIT_LIMIT",
            "requested Git output limit exceeds the native artifact limit",
        ));
    }
    #[cfg(unix)]
    {
        unix::git_bounded(
            root,
            &args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>(),
            false,
            limit,
        )
        .map(|(_, bytes)| bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, args, limit);
        Err(unsupported())
    }
}

/// Read effective Git policy without executing it. Only the fixed config-list
/// command sees standard global/system settings and GIT_CONFIG_* overrides.
pub(crate) fn git_config_for_completion(root: &Path) -> Result<Vec<u8>> {
    #[cfg(unix)]
    {
        unix::git_config(root).map(|(_, bytes)| bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err(unsupported())
    }
}

/// Snapshot-only reads preserve an explicitly captured Git repository/index
/// context. Callers supply only fixed read commands, never user command text.
pub(crate) fn git_snapshot_read(
    root: &Path,
    args: &[std::ffi::OsString],
    context: &[(std::ffi::OsString, std::ffi::OsString)],
    allow_one: bool,
    limit: usize,
) -> Result<(bool, Vec<u8>)> {
    if limit > crate::native_bundle::MAX_PAYLOAD_BYTES {
        return Err(fail(
            "WORKSPACE_GIT_LIMIT",
            "snapshot Git output limit exceeds native artifact budget",
        ));
    }
    #[cfg(unix)]
    {
        unix::git_snapshot(
            root,
            &args.iter().map(|a| a.as_os_str()).collect::<Vec<_>>(),
            context,
            allow_one,
            limit,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = (root, args, context, allow_one, limit);
        Err(unsupported())
    }
}

/// Only destination-local identifiers and hashes; no prompts or executable settings.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkerMetadata {
    #[serde(default)]
    pub context_initialized: bool,
    #[serde(default)]
    pub staging_in_progress: bool,
    /// Observed native-history requirement; the launcher must still require explicit routing.
    #[serde(default)]
    pub requires_tool_rebinding: bool,
    pub launch_id: Option<String>,
    pub thread_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub selection_digest: Option<String>,
    pub seed_bundle_sha256: Option<String>,
    pub saved_bundle_sha256: Option<String>,
    /// The selected projection can differ from the most recently named export.
    #[serde(default)]
    pub selected_bundle_sha256: Option<String>,
    /// Distinguish a recorded fresh selection from an older receipt without an anchor.
    #[serde(default)]
    pub selected_bundle_recorded: bool,
    /// Durable acknowledgement of initial staging before app-server shutdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staged_worker: Option<crate::recovery::StagedWorker>,
    /// Exact completed capture awaiting publication bookkeeping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_save: Option<crate::recovery::PendingSave>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_recovery_sha256: Option<String>,
}

impl WorkerMetadata {
    pub fn accepts_selected_bundle(&self, hash: Option<&str>) -> bool {
        if self.thread_id.is_none() {
            return true;
        }
        if self.selected_bundle_recorded {
            hash == self.selected_bundle_sha256.as_deref()
        } else {
            // Receipts predating the selection anchor are upgraded on a successful
            // resume/save, using only their previously recorded seed or export.
            hash == self.seed_bundle_sha256.as_deref()
                || self
                    .saved_bundle_sha256
                    .as_deref()
                    .is_some_and(|saved| hash == Some(saved))
        }
    }

    pub fn validate(&self) -> Result<()> {
        let uuid = |s: &str| {
            s.len() == 36
                && s.bytes().enumerate().all(|(i, b)| {
                    if matches!(i, 8 | 13 | 18 | 23) {
                        b == b'-'
                    } else {
                        b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
                    }
                })
                && s.bytes().any(|b| b != b'0' && b != b'-')
        };
        let runtime_name = |s: &str| {
            !s.is_empty()
                && s.len() <= 256
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
        };
        if self.launch_id.as_deref().is_some_and(|s| !hex(s, 32))
            || self.thread_id.as_deref().is_some_and(|s| !uuid(s))
            || [&self.model, &self.provider]
                .into_iter()
                .flatten()
                .any(|s| !runtime_name(s))
            || [
                &self.selection_digest,
                &self.seed_bundle_sha256,
                &self.saved_bundle_sha256,
                &self.selected_bundle_sha256,
                &self.last_recovery_sha256,
            ]
            .into_iter()
            .flatten()
            .any(|s| !hex(s, 64))
        {
            return Err(fail(
                "WORKSPACE_METADATA",
                "invalid worker identifiers or digests",
            ));
        }
        if let Some(staged) = &self.staged_worker {
            staged.validate()?;
            if !self.context_initialized
                || !self.staging_in_progress
                || self
                    .thread_id
                    .as_deref()
                    .is_some_and(|id| id != staged.thread_id)
            {
                return Err(fail(
                    "WORKSPACE_METADATA",
                    "staging evidence conflicts with the worker lifecycle",
                ));
            }
        }
        if let Some(save) = &self.pending_save {
            save.validate()?;
            if !self.context_initialized
                || self.thread_id.is_none()
                || self.staging_in_progress
                || !self.requires_tool_rebinding
            {
                return Err(fail(
                    "WORKSPACE_METADATA",
                    "save evidence conflicts with the worker lifecycle",
                ));
            }
        }
        if self.staged_worker.is_some() && self.pending_save.is_some() {
            return Err(fail(
                "WORKSPACE_METADATA",
                "staging and save evidence cannot overlap",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    Planned,
    Preparing,
    Ready,
    Failed,
}

/// A no-write observation. Existing incomplete receipts are reported, never adopted.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspacePlan {
    pub root: PathBuf,
    pub branch: String,
    pub base_commit: String,
    pub repository_id: String,
    pub work_id: String,
    pub existing: bool,
    pub status: BindingStatus,
    pub worker_metadata: WorkerMetadata,
    pub last_error: Option<String>,
}

/// Holds exclusive ownership of this work ID until dropped; never removes files.
pub struct WorktreeBinding {
    receipt: Receipt,
    proxy_path: PathBuf,
    newly_created: bool,
    #[cfg(unix)]
    storage: unix::Storage,
}

impl WorktreeBinding {
    pub fn newly_created(&self) -> bool {
        self.newly_created
    }
    /// Uses ASTRAL_WORKTREE_ROOT only as trusted process configuration, if present.
    pub fn acquire(
        source_root: &Path,
        project_id: &str,
        selector: &str,
        work_id: &str,
    ) -> Result<Self> {
        Self::acquire_with_root(
            source_root,
            project_id,
            selector,
            work_id,
            configured_root()?.as_deref(),
        )
    }

    /// Explicit host override. None selects the sibling .astral-worktrees directory.
    pub fn acquire_with_root(
        source_root: &Path,
        project_id: &str,
        selector: &str,
        work_id: &str,
        managed_root: Option<&Path>,
    ) -> Result<Self> {
        #[cfg(unix)]
        {
            unix::acquire(
                source_root,
                project_id,
                selector,
                work_id,
                managed_root,
                true,
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (source_root, project_id, selector, work_id, managed_root);
            Err(unsupported())
        }
    }

    /// Bound-only lookup for save: does not create directories, lock files or worktrees.
    pub fn open_existing(
        source_root: &Path,
        project_id: &str,
        selector: &str,
        work_id: &str,
    ) -> Result<Self> {
        Self::open_existing_with_root(
            source_root,
            project_id,
            selector,
            work_id,
            configured_root()?.as_deref(),
        )
    }

    pub fn open_existing_with_root(
        source_root: &Path,
        project_id: &str,
        selector: &str,
        work_id: &str,
        managed_root: Option<&Path>,
    ) -> Result<Self> {
        #[cfg(unix)]
        {
            unix::acquire(
                source_root,
                project_id,
                selector,
                work_id,
                managed_root,
                false,
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (source_root, project_id, selector, work_id, managed_root);
            Err(unsupported())
        }
    }

    /// No filesystem writes or ownership locks, including when no binding exists.
    pub fn inspect(
        source_root: &Path,
        project_id: &str,
        selector: &str,
        work_id: &str,
    ) -> Result<WorkspacePlan> {
        Self::inspect_with_root(
            source_root,
            project_id,
            selector,
            work_id,
            configured_root()?.as_deref(),
        )
    }

    pub fn inspect_with_root(
        source_root: &Path,
        project_id: &str,
        selector: &str,
        work_id: &str,
        managed_root: Option<&Path>,
    ) -> Result<WorkspacePlan> {
        #[cfg(unix)]
        {
            unix::inspect(source_root, project_id, selector, work_id, managed_root)
        }
        #[cfg(not(unix))]
        {
            let _ = (source_root, project_id, selector, work_id, managed_root);
            Err(unsupported())
        }
    }

    pub fn root(&self) -> &Path {
        &self.receipt.root
    }
    pub fn branch(&self) -> &str {
        &self.receipt.branch
    }
    pub fn base_commit(&self) -> &str {
        &self.receipt.base_commit
    }
    pub fn repository_id(&self) -> &str {
        &self.receipt.repository_id
    }
    pub fn work_id(&self) -> &str {
        &self.receipt.work_id
    }
    pub fn worker_metadata(&self) -> &WorkerMetadata {
        &self.receipt.worker_metadata
    }
    /// Private owned proxy storage for the launcher; never a portable project option.
    pub fn proxy_state_dir(&self) -> &Path {
        &self.proxy_path
    }

    /// Atomic metadata replacement, with validation and an optimistic receipt check.
    pub fn update_worker_metadata(&mut self, metadata: WorkerMetadata) -> Result<()> {
        metadata.validate()?;
        #[cfg(unix)]
        {
            unix::validate_live(&self.receipt)?;
            let mut next = self.receipt.clone();
            next.worker_metadata = metadata;
            self.storage.write(&next)?;
            self.receipt = next;
            Ok(())
        }
        #[cfg(not(unix))]
        {
            Err(unsupported())
        }
    }
}

#[cfg(not(unix))]
fn unsupported() -> Error {
    fail(
        "WORKSPACE_UNSUPPORTED_PLATFORM",
        "worktree bindings currently require Unix",
    )
}

fn configured_root() -> Result<Option<PathBuf>> {
    std::env::var_os("ASTRAL_WORKTREE_ROOT")
        .map(PathBuf::from)
        .map(|p| {
            if p.is_absolute() {
                Ok(p)
            } else {
                Err(fail(
                    "WORKSPACE_PATH",
                    "ASTRAL_WORKTREE_ROOT must be absolute",
                ))
            }
        })
        .transpose()
}

fn fail(code: &'static str, message: &str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn hex(s: &str, len: usize) -> bool {
    s.len() == len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    schema_version: u32,
    repository_id: String,
    common_dir: PathBuf,
    common_identity: FileIdentity,
    project_id: String,
    selector: String,
    work_id: String,
    root: PathBuf,
    branch: String,
    base_commit: String,
    status: BindingStatus,
    root_identity: Option<FileIdentity>,
    git_dir: Option<PathBuf>,
    git_identity: Option<FileIdentity>,
    worker_metadata: WorkerMetadata,
    last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    creation_proof: Option<recovery::CreationProof>,
}

#[cfg(unix)]
impl Receipt {
    fn plan(&self, existing: bool) -> WorkspacePlan {
        WorkspacePlan {
            root: self.root.clone(),
            branch: self.branch.clone(),
            base_commit: self.base_commit.clone(),
            repository_id: self.repository_id.clone(),
            work_id: self.work_id.clone(),
            existing,
            status: self.status,
            worker_metadata: self.worker_metadata.clone(),
            last_error: self.last_error.clone(),
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::ffi::{CString, OsStr};
    use std::fs::{File, Metadata, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::{Duration, Instant};

    const GIT_TIMEOUT: Duration = Duration::from_secs(30);
    const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
    const MAX_STDOUT: usize = 1_048_576;
    const MAX_STDERR: usize = 65_536;
    const MAX_WORKTREES: usize = 4096;

    pub(super) fn io_error(_: std::io::Error) -> Error {
        fail("WORKSPACE_IO", "workspace filesystem operation failed")
    }
    pub(super) fn identity(m: &Metadata) -> FileIdentity {
        FileIdentity {
            device: m.dev(),
            inode: m.ino(),
        }
    }
    fn uid() -> u32 {
        unsafe { libc::geteuid() }
    }

    fn path_valid(path: &Path) -> Result<()> {
        let b = path.as_os_str().as_bytes();
        if !path.is_absolute()
            || b.len() > 4096
            || path.to_str().is_none()
            || b.contains(&0)
            || b.split(|b| *b == b'/')
                .skip(1)
                .any(|p| p.is_empty() || p == b"." || p == b"..")
        {
            return Err(fail(
                "WORKSPACE_PATH",
                "workspace paths must be bounded absolute UTF-8 paths without dot components",
            ));
        }
        Ok(())
    }

    fn name(s: &OsStr) -> Result<CString> {
        CString::new(s.as_bytes()).map_err(|_| fail("WORKSPACE_PATH", "path contains NUL"))
    }

    pub(super) fn open_at(
        parent: &File,
        part: &OsStr,
        directory: bool,
        create: bool,
        writable: bool,
    ) -> Result<Option<File>> {
        let part = name(part)?;
        let flags = libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory {
                libc::O_RDONLY | libc::O_DIRECTORY
            } else if writable {
                libc::O_RDWR
            } else {
                libc::O_RDONLY
            }
            | if create {
                libc::O_CREAT | libc::O_EXCL
            } else {
                0
            };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), part.as_ptr(), flags, 0o600) };
        if fd >= 0 {
            return Ok(Some(unsafe { File::from_raw_fd(fd) }));
        }
        let e = std::io::Error::last_os_error();
        if !create && e.kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        Err(fail(
            "WORKSPACE_UNSAFE_PATH",
            "workspace entry is unsafe or unavailable",
        ))
    }

    fn mkdir_at(parent: &File, part: &OsStr) -> Result<()> {
        let part = name(part)?;
        if unsafe { libc::mkdirat(parent.as_raw_fd(), part.as_ptr(), 0o700) } == 0 {
            return Ok(());
        }
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists {
            return Ok(());
        }
        Err(fail("WORKSPACE_IO", "cannot create workspace directory"))
    }

    pub(super) fn secure(m: &Metadata, dir: bool) -> Result<()> {
        if m.uid() != uid()
            || m.mode() & 0o7777 != if dir { 0o700 } else { 0o600 }
            || if dir {
                !m.is_dir()
            } else {
                !m.is_file() || m.nlink() != 1
            }
        {
            return Err(fail(
                "WORKSPACE_PRIVATE_STATE",
                "binding storage must be private, owned, and of the expected file type",
            ));
        }
        Ok(())
    }

    // Every component is opened relative to a pinned descriptor, never following links.
    fn directory(path: &Path, create: bool) -> Result<Option<File>> {
        path_valid(path)?;
        let mut dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")
            .map_err(io_error)?;
        for part in path.components().skip(1) {
            if create {
                mkdir_at(&dir, part.as_os_str())?;
            }
            let Some(next) = open_at(&dir, part.as_os_str(), true, false, false)? else {
                return Ok(None);
            };
            let m = next.metadata().map_err(io_error)?;
            if ![0, uid()].contains(&m.uid()) || (m.mode() & 0o022 != 0 && m.mode() & 0o1000 == 0) {
                return Err(fail(
                    "WORKSPACE_UNSAFE_PATH",
                    "workspace ancestor allows unsafe replacement",
                ));
            }
            dir = next;
        }
        Ok(Some(dir))
    }

    pub(super) fn required_dir(path: &Path) -> Result<File> {
        directory(path, false)?
            .ok_or_else(|| fail("WORKSPACE_MISSING", "bound workspace directory is missing"))
    }

    pub(super) fn private_child(parent: &File, part: &str, create: bool) -> Result<Option<File>> {
        if create {
            mkdir_at(parent, OsStr::new(part))?;
        }
        let file = open_at(parent, OsStr::new(part), true, false, false)?;
        if let Some(file) = &file {
            secure(&file.metadata().map_err(io_error)?, true)?;
        }
        Ok(file)
    }

    pub(super) fn store(common: &Path, create: bool) -> Result<Option<File>> {
        let common = required_dir(common)?;
        let Some(astral) = private_child(&common, "astral", create)? else {
            return Ok(None);
        };
        private_child(&astral, "work-bindings", create)
    }

    pub(super) fn lock(parent: &File, part: &str, create: bool, wait: bool) -> Result<File> {
        let file = match open_at(parent, OsStr::new(part), false, false, true)? {
            Some(file) => file,
            None if create => match open_at(parent, OsStr::new(part), false, true, true) {
                Ok(Some(file)) => file,
                _ => open_at(parent, OsStr::new(part), false, false, true)?
                    .ok_or_else(|| fail("WORKSPACE_LOCK", "lock creation failed"))?,
            },
            None => {
                return Err(fail(
                    "WORKSPACE_INCOMPLETE",
                    "binding ownership lock is missing",
                ));
            }
        };
        secure(&file.metadata().map_err(io_error)?, false)?;
        let start = Instant::now();
        loop {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => return Ok(file),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if !wait || start.elapsed() >= LOCK_TIMEOUT {
                        return Err(fail(
                            if wait {
                                "WORKSPACE_REPOSITORY_BUSY"
                            } else {
                                "WORKSPACE_OWNED"
                            },
                            "another process holds workspace ownership",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return Err(fail("WORKSPACE_LOCK", "cannot acquire workspace ownership")),
            }
        }
    }

    struct GitChild(Child, bool);
    impl Drop for GitChild {
        fn drop(&mut self) {
            // The child owns a new process group, also bounding inherited pipe writers.
            if self.1 {
                unsafe {
                    libc::kill(-(self.0.id() as i32), libc::SIGKILL);
                }
            } else {
                let _ = self.0.kill();
            }
            let _ = self.0.wait();
        }
    }

    fn nonblocking(file: &impl AsRawFd) -> Result<()> {
        let fd = file.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(fail("WORKSPACE_GIT_IO", "cannot bound Git output"));
        }
        Ok(())
    }

    fn drain(stream: &mut impl Read, buf: &mut Vec<u8>, limit: usize) -> Result<bool> {
        // Limit each drain as well as total bytes, so a noisy stream cannot starve timeout checks.
        let mut chunk = [0u8; 8192];
        for _ in 0..16 {
            match stream.read(&mut chunk) {
                Ok(0) => return Ok(true),
                Ok(n) => {
                    if buf.len() + n > limit {
                        return Err(fail(
                            "WORKSPACE_GIT_LIMIT",
                            "Git output exceeded its bounded limit",
                        ));
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(fail("WORKSPACE_GIT_IO", "cannot read Git output")),
            }
        }
        Ok(false)
    }

    pub(super) fn git(root: &Path, args: &[&OsStr], allow_one: bool) -> Result<(bool, Vec<u8>)> {
        git_bounded(root, args, allow_one, MAX_STDOUT)
    }

    pub(super) fn git_bounded(
        root: &Path,
        args: &[&OsStr],
        allow_one: bool,
        stdout_limit: usize,
    ) -> Result<(bool, Vec<u8>)> {
        git_run(root, args, allow_one, stdout_limit, false, &[])
    }

    pub(super) fn git_config(root: &Path) -> Result<(bool, Vec<u8>)> {
        git_run(
            root,
            &["config", "--null", "--list", "--includes"].map(OsStr::new),
            false,
            MAX_STDOUT,
            true,
            &[],
        )
    }

    pub(super) fn git_snapshot(
        root: &Path,
        args: &[&OsStr],
        context: &[(std::ffi::OsString, std::ffi::OsString)],
        allow_one: bool,
        limit: usize,
    ) -> Result<(bool, Vec<u8>)> {
        git_run(root, args, allow_one, limit, false, context)
    }

    fn git_run(
        root: &Path,
        args: &[&OsStr],
        allow_one: bool,
        stdout_limit: usize,
        observe_config: bool,
        snapshot_context: &[(std::ffi::OsString, std::ffi::OsString)],
    ) -> Result<(bool, Vec<u8>)> {
        let mut cmd = Command::new("git");
        cmd.current_dir(root).args([
            "--no-pager",
            "--no-optional-locks",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "submodule.recurse=false",
            "-c",
            "gc.auto=0",
            "-c",
            "maintenance.auto=false",
        ]);
        if !observe_config {
            cmd.args(["-c", "core.hooksPath=/dev/null"]);
        }
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let owns_group = std::env::var_os("ASTRAL_HOOK_CHILD").as_deref() != Some(OsStr::new("1"));
        if owns_group {
            cmd.process_group(0);
        }
        for (key, _) in std::env::vars_os() {
            if key.as_bytes().starts_with(b"GIT_")
                && !(observe_config && key.as_bytes().starts_with(b"GIT_CONFIG_"))
            {
                cmd.env_remove(key);
            }
        }
        for (key, value) in snapshot_context {
            cmd.env(key, value);
        }
        if !snapshot_context.is_empty() {
            cmd.env("GIT_NO_LAZY_FETCH", "1")
                .env("GIT_NO_REPLACE_OBJECTS", "1")
                .env("GIT_LITERAL_PATHSPECS", "1");
        }
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_ATTR_NOSYSTEM", "1")
            .env("LC_ALL", "C");
        if !observe_config && snapshot_context.is_empty() {
            cmd.env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null");
        }
        let mut child = GitChild(
            cmd.spawn()
                .map_err(|_| fail("WORKSPACE_GIT_START", "cannot start Git"))?,
            owns_group,
        );
        let mut stdout = child.0.stdout.take().expect("piped stdout");
        let mut stderr = child.0.stderr.take().expect("piped stderr");
        nonblocking(&stdout)?;
        nonblocking(&stderr)?;
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let start = Instant::now();
        let mut status = None;
        loop {
            let out_done = drain(&mut stdout, &mut out, stdout_limit)?;
            let err_done = drain(&mut stderr, &mut err, MAX_STDERR)?;
            if status.is_none() {
                status = child.0.try_wait().map_err(io_error)?;
            }
            if let Some(status) = status {
                if out_done && err_done {
                    if status.success() {
                        return Ok((true, out));
                    }
                    if allow_one && status.code() == Some(1) {
                        return Ok((false, out));
                    }
                    return Err(fail(
                        "WORKSPACE_GIT_FAILED",
                        "Git command failed; any incomplete binding and Git artifacts were retained",
                    ));
                }
            }
            if start.elapsed() >= GIT_TIMEOUT {
                return Err(fail(
                    "WORKSPACE_GIT_TIMEOUT",
                    "Git command exceeded 30 seconds; incomplete artifacts were retained",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn git_text(root: &Path, args: &[&str]) -> Result<String> {
        let args: Vec<_> = args.iter().map(OsStr::new).collect();
        let (_, out) = git(root, &args, false)?;
        let out = out.strip_suffix(b"\n").unwrap_or(&out);
        if out.len() > 4096 || out.contains(&0) {
            return Err(fail("WORKSPACE_GIT_FORMAT", "invalid Git plumbing output"));
        }
        String::from_utf8(out.to_vec())
            .map_err(|_| fail("WORKSPACE_GIT_FORMAT", "Git paths require UTF-8"))
    }

    struct GitWorktree {
        root: PathBuf,
        branch: Option<String>,
        head: Option<String>,
        bare: bool,
    }

    fn worktrees(root: &Path) -> Result<Vec<GitWorktree>> {
        let (_, out) = git(
            root,
            &[
                OsStr::new("worktree"),
                OsStr::new("list"),
                OsStr::new("--porcelain"),
                OsStr::new("-z"),
            ],
            false,
        )?;
        if !out.ends_with(b"\0\0") {
            return Err(fail(
                "WORKSPACE_GIT_FORMAT",
                "invalid worktree porcelain output",
            ));
        }
        let mut entries = Vec::new();
        let mut current: Option<GitWorktree> = None;
        for field in out.split(|b| *b == 0) {
            if field.is_empty() {
                if let Some(entry) = current.take() {
                    entries.push(entry);
                }
                continue;
            }
            if field.len() > 8192 {
                return Err(fail("WORKSPACE_GIT_LIMIT", "worktree field is too large"));
            }
            if let Some(path) = field.strip_prefix(b"worktree ") {
                if current.is_some() || entries.len() >= MAX_WORKTREES {
                    return Err(fail(
                        "WORKSPACE_GIT_LIMIT",
                        "invalid or excessive worktree entries",
                    ));
                }
                current = Some(GitWorktree {
                    root: PathBuf::from(OsStr::from_bytes(path)),
                    branch: None,
                    head: None,
                    bare: false,
                });
            } else if let Some(entry) = &mut current {
                if let Some(branch) = field.strip_prefix(b"branch ") {
                    entry.branch =
                        Some(String::from_utf8(branch.to_vec()).map_err(|_| {
                            fail("WORKSPACE_GIT_FORMAT", "invalid branch encoding")
                        })?);
                } else if let Some(head) = field.strip_prefix(b"HEAD ") {
                    entry.head =
                        Some(String::from_utf8(head.to_vec()).map_err(|_| {
                            fail("WORKSPACE_GIT_FORMAT", "invalid commit encoding")
                        })?);
                } else if field == b"bare" {
                    entry.bare = true;
                }
            } else {
                return Err(fail("WORKSPACE_GIT_FORMAT", "invalid worktree record"));
            }
        }
        Ok(entries)
    }

    fn logical(s: &str) -> bool {
        !s.is_empty()
            && s.len() <= 128
            && s != "."
            && s != ".."
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    }

    pub(super) fn selection_valid(selector: &str) -> bool {
        selector.split_once(':').map_or_else(
            || logical(selector),
            |(kind, id)| matches!(kind, "subsystem" | "projection") && logical(id),
        )
    }

    pub(super) fn expected(
        source: &Path,
        project: &str,
        selector: &str,
        work: &str,
        managed: Option<&Path>,
    ) -> Result<Receipt> {
        if !logical(project)
            || !selection_valid(selector)
            || !logical(work)
            || work.starts_with('.')
            || work.ends_with('.')
            || work.ends_with(".lock")
            || work.contains("..")
        {
            return Err(fail(
                "WORKSPACE_IDENTITY",
                "invalid project, selection or work identifier",
            ));
        }
        let source = source.canonicalize().map_err(io_error)?;
        required_dir(&source)?;
        let root = PathBuf::from(git_text(
            &source,
            &["rev-parse", "--path-format=absolute", "--show-toplevel"],
        )?);
        required_dir(&root)?;
        if !source.starts_with(&root) {
            return Err(fail(
                "WORKSPACE_REPOSITORY",
                "Git's checkout root does not contain the invoking directory",
            ));
        }
        let common = PathBuf::from(git_text(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?);
        let common = common.canonicalize().map_err(io_error)?;
        let common_file = required_dir(&common)?;
        let base = git_text(&root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        if !(hex(&base, 40) || hex(&base, 64)) {
            return Err(fail("WORKSPACE_GIT_FORMAT", "invalid HEAD commit"));
        }
        let repository_id = crate::hash(common.as_os_str().as_bytes());
        let entries = worktrees(&root)?;
        let main = entries.first().filter(|e| !e.bare).ok_or_else(|| {
            fail(
                "WORKSPACE_REPOSITORY",
                "a main non-bare checkout is required",
            )
        })?;
        let main_root = main.root.canonicalize().map_err(io_error)?;
        required_dir(&main_root)?;
        let managed = managed.map(Path::to_path_buf).unwrap_or_else(|| {
            main_root
                .parent()
                .unwrap_or(Path::new("/"))
                .join(".astral-worktrees")
        });
        path_valid(&managed)?;
        let destination = managed.join(&repository_id).join(work);
        path_valid(&destination)?;
        directory(&managed, false)?;
        if managed.starts_with(&common)
            || entries
                .iter()
                .any(|e| !e.bare && managed.starts_with(&e.root))
        {
            return Err(fail(
                "WORKSPACE_PATH",
                "managed worktrees must be outside existing checkouts and the Git common directory",
            ));
        }
        Ok(Receipt {
            schema_version: 1,
            repository_id,
            common_dir: common,
            common_identity: identity(&common_file.metadata().map_err(io_error)?),
            project_id: project.into(),
            selector: selector.into(),
            work_id: work.into(),
            root: destination,
            branch: format!("astral/{work}"),
            base_commit: base,
            status: BindingStatus::Planned,
            root_identity: None,
            git_dir: None,
            git_identity: None,
            worker_metadata: WorkerMetadata::default(),
            last_error: None,
            creation_proof: None,
        })
    }

    pub(super) fn collision(receipt: &Receipt) -> Result<()> {
        // lstat catches a dangling symlink too; an empty directory is still a conflict.
        match std::fs::symlink_metadata(&receipt.root) {
            Ok(_) => {
                return Err(fail(
                    "WORKSPACE_CONFLICT",
                    "planned worktree path already exists without a matching binding",
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            Err(e) => return Err(io_error(e)),
        }
        directory(receipt.root.parent().expect("absolute managed path"), false)?;
        let branch = format!("refs/heads/{}", receipt.branch);
        let (exists, _) = git(
            &receipt.common_dir,
            &[
                OsStr::new("show-ref"),
                OsStr::new("--verify"),
                OsStr::new("--quiet"),
                OsStr::new(&branch),
            ],
            true,
        )?;
        if exists
            || worktrees(&receipt.common_dir)?
                .iter()
                .any(|e| e.root == receipt.root || e.branch.as_deref() == Some(branch.as_str()))
        {
            return Err(fail(
                "WORKSPACE_CONFLICT",
                "planned branch or worktree registration already exists without a matching binding",
            ));
        }
        Ok(())
    }

    pub(super) fn read_receipt(dir: &File) -> Result<Vec<u8>> {
        let file =
            open_at(dir, OsStr::new("receipt.json"), false, false, false)?.ok_or_else(|| {
                fail(
                    "WORKSPACE_INCOMPLETE",
                    "binding directory has no receipt; incomplete creation was retained",
                )
            })?;
        let m = file.metadata().map_err(io_error)?;
        secure(&m, false)?;
        if m.len() > MAX_BINDING_BYTES as u64 {
            return Err(fail(
                "WORKSPACE_METADATA_LIMIT",
                "binding receipt exceeds its size limit",
            ));
        }
        let mut bytes = Vec::new();
        file.take(MAX_BINDING_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() > MAX_BINDING_BYTES {
            return Err(fail(
                "WORKSPACE_METADATA_LIMIT",
                "binding receipt exceeds its size limit",
            ));
        }
        Ok(bytes)
    }

    pub(super) fn parse(bytes: &[u8], expected: &Receipt) -> Result<Receipt> {
        let receipt: Receipt = serde_json::from_slice(bytes)
            .map_err(|_| fail("WORKSPACE_METADATA", "binding JSON or schema is invalid"))?;
        receipt.worker_metadata.validate()?;
        if receipt.schema_version != 1
            || receipt.repository_id != expected.repository_id
            || receipt.common_dir != expected.common_dir
            || receipt.common_identity != expected.common_identity
            || receipt.project_id != expected.project_id
            || receipt.selector != expected.selector
            || receipt.work_id != expected.work_id
            || receipt.root != expected.root
            || receipt.branch != expected.branch
            || !(hex(&receipt.base_commit, 40) || hex(&receipt.base_commit, 64))
        {
            return Err(fail(
                "WORKSPACE_MISMATCH",
                "binding identity differs from the repository, path, project, selector or work ID",
            ));
        }
        let ready = receipt.root_identity.is_some()
            && receipt.git_dir.is_some()
            && receipt.git_identity.is_some();
        let empty = receipt.root_identity.is_none()
            && receipt.git_dir.is_none()
            && receipt.git_identity.is_none();
        if receipt.status == BindingStatus::Planned
            || (receipt.status == BindingStatus::Ready) != ready
            || receipt.status != BindingStatus::Ready && !empty
            || (receipt.status == BindingStatus::Failed) != receipt.last_error.is_some()
            || receipt.last_error.as_deref().is_some_and(|s| {
                s.is_empty()
                    || s.len() > 96
                    || !s.bytes().all(|b| b.is_ascii_uppercase() || b == b'_')
            })
            || receipt.status != BindingStatus::Ready
                && receipt.worker_metadata != WorkerMetadata::default()
        {
            return Err(fail(
                "WORKSPACE_METADATA",
                "binding lifecycle metadata is invalid",
            ));
        }
        if let Some(git_dir) = &receipt.git_dir {
            path_valid(git_dir)?;
            if git_dir.parent() != Some(receipt.common_dir.join("worktrees").as_path()) {
                return Err(fail(
                    "WORKSPACE_MISMATCH",
                    "bound Git directory is outside this repository's worktree registry",
                ));
            }
        }
        if let Some(proof) = &receipt.creation_proof {
            path_valid(&proof.git_dir)?;
            if proof.schema_version != 1
                || proof.git_dir.parent() != Some(receipt.common_dir.join("worktrees").as_path())
                || receipt.status == BindingStatus::Ready
                    && (receipt.root_identity != Some(proof.root_identity)
                        || receipt.git_dir.as_ref() != Some(&proof.git_dir)
                        || receipt.git_identity != Some(proof.git_identity))
            {
                return Err(fail(
                    "WORKSPACE_METADATA",
                    "checkout identity proof differs from the bound worktree",
                ));
            }
        }
        Ok(receipt)
    }

    pub(super) fn validate_live(receipt: &Receipt) -> Result<()> {
        if identity(
            &required_dir(&receipt.common_dir)?
                .metadata()
                .map_err(io_error)?,
        ) != receipt.common_identity
            || Some(identity(
                &required_dir(&receipt.root)?.metadata().map_err(io_error)?,
            )) != receipt.root_identity
        {
            return Err(fail(
                "WORKSPACE_MISMATCH",
                "repository or bound worktree directory was replaced",
            ));
        }
        let git_dir = receipt
            .git_dir
            .as_ref()
            .ok_or_else(|| fail("WORKSPACE_INCOMPLETE", "worktree creation is incomplete"))?;
        if Some(identity(
            &required_dir(git_dir)?.metadata().map_err(io_error)?,
        )) != receipt.git_identity
        {
            return Err(fail(
                "WORKSPACE_MISMATCH",
                "bound Git worktree directory was replaced",
            ));
        }
        let dot_git = receipt.root.join(".git");
        let m = std::fs::symlink_metadata(dot_git).map_err(io_error)?;
        if !m.is_file() || m.nlink() != 1 || m.len() > 4096 {
            return Err(fail(
                "WORKSPACE_MISMATCH",
                "bound worktree .git file is invalid",
            ));
        }
        let common = PathBuf::from(git_text(
            &receipt.root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?);
        let actual_git = PathBuf::from(git_text(
            &receipt.root,
            &["rev-parse", "--absolute-git-dir"],
        )?);
        let actual_root = PathBuf::from(git_text(
            &receipt.root,
            &["rev-parse", "--path-format=absolute", "--show-toplevel"],
        )?);
        required_dir(&common)?;
        required_dir(&actual_git)?;
        if common != receipt.common_dir || actual_git != *git_dir || actual_root != receipt.root {
            return Err(fail(
                "WORKSPACE_MISMATCH",
                "actual Git directories differ from the binding",
            ));
        }
        let branch = format!("refs/heads/{}", receipt.branch);
        if git_text(&receipt.root, &["symbolic-ref", "--quiet", "HEAD"])? != branch {
            return Err(fail(
                "WORKSPACE_BRANCH_MISMATCH",
                "bound worktree is on a different branch",
            ));
        }
        let entries = worktrees(&receipt.common_dir)?;
        let matching: Vec<_> = entries
            .iter()
            .filter(|e| e.root == receipt.root || e.branch.as_deref() == Some(&branch))
            .collect();
        let head = git_text(&receipt.root, &["rev-parse", "--verify", "HEAD^{commit}"])?;
        if matching.len() != 1
            || matching[0].root != receipt.root
            || matching[0].branch.as_deref() != Some(&branch)
            || matching[0].head.as_deref() != Some(&head)
        {
            return Err(fail(
                "WORKSPACE_BRANCH_MISMATCH",
                "Git worktree registration or branch ownership differs from the binding",
            ));
        }
        let (descendant, _) = git(
            &receipt.root,
            &[
                OsStr::new("merge-base"),
                OsStr::new("--is-ancestor"),
                OsStr::new(&receipt.base_commit),
                OsStr::new(&head),
            ],
            true,
        )?;
        if !descendant {
            return Err(fail(
                "WORKSPACE_BRANCH_MISMATCH",
                "bound branch no longer descends from its recorded base",
            ));
        }
        Ok(())
    }

    pub(super) struct Storage {
        pub(super) directory: File,
        pub(super) lock: File,
        pub(super) path: PathBuf,
        pub(super) expected_hash: Option<String>,
    }

    impl Drop for Storage {
        fn drop(&mut self) {
            let _ = fs2::FileExt::unlock(&self.lock);
        }
    }

    impl Storage {
        pub(super) fn write(&mut self, receipt: &Receipt) -> Result<()> {
            let live_dir = required_dir(&self.path)?;
            let live_lock = open_at(&live_dir, OsStr::new("owner.lock"), false, false, false)?
                .ok_or_else(|| fail("WORKSPACE_CHANGED", "binding ownership file disappeared"))?;
            secure(&live_dir.metadata().map_err(io_error)?, true)?;
            secure(&live_lock.metadata().map_err(io_error)?, false)?;
            if identity(&live_dir.metadata().map_err(io_error)?)
                != identity(&self.directory.metadata().map_err(io_error)?)
                || identity(&live_lock.metadata().map_err(io_error)?)
                    != identity(&self.lock.metadata().map_err(io_error)?)
            {
                return Err(fail(
                    "WORKSPACE_CHANGED",
                    "binding directory or ownership file changed while held",
                ));
            }
            if let Some(expected) = &self.expected_hash {
                if crate::hash(&read_receipt(&self.directory)?) != *expected {
                    return Err(fail(
                        "WORKSPACE_CHANGED",
                        "binding receipt changed while owned",
                    ));
                }
            } else if open_at(
                &self.directory,
                OsStr::new("receipt.json"),
                false,
                false,
                false,
            )?
            .is_some()
            {
                return Err(fail(
                    "WORKSPACE_CHANGED",
                    "binding receipt appeared during creation",
                ));
            }
            let bytes = serde_json::to_vec(receipt)
                .map_err(|_| fail("WORKSPACE_METADATA", "cannot serialize binding"))?;
            if bytes.len() > MAX_BINDING_BYTES {
                return Err(fail(
                    "WORKSPACE_METADATA_LIMIT",
                    "binding receipt exceeds its size limit",
                ));
            }
            let mut random = [0u8; 16];
            getrandom::fill(&mut random)
                .map_err(|_| fail("WORKSPACE_ENTROPY", "OS randomness is unavailable"))?;
            let temp = format!("receipt-{}.tmp", crate::hash(&random));
            let mut file = open_at(&self.directory, OsStr::new(&temp), false, true, true)?
                .expect("created file");
            file.write_all(&bytes).map_err(io_error)?;
            file.sync_all().map_err(io_error)?;
            let from = name(OsStr::new(&temp))?;
            let to = name(OsStr::new("receipt.json"))?;
            if unsafe {
                libc::renameat(
                    self.directory.as_raw_fd(),
                    from.as_ptr(),
                    self.directory.as_raw_fd(),
                    to.as_ptr(),
                )
            } != 0
            {
                return Err(io_error(std::io::Error::last_os_error()));
            }
            self.expected_hash = Some(crate::hash(&bytes));
            self.directory.sync_all().map_err(io_error)?;
            Ok(())
        }
    }

    pub(super) fn binding_path(receipt: &Receipt) -> PathBuf {
        receipt
            .common_dir
            .join("astral/work-bindings")
            .join(&receipt.work_id)
    }

    fn existing(dir: File, expected: &Receipt) -> Result<WorktreeBinding> {
        let lock = lock(&dir, "owner.lock", false, false)?;
        let bytes = read_receipt(&dir)?;
        let receipt = parse(&bytes, expected)?;
        if receipt.status != BindingStatus::Ready {
            return Err(fail(
                "WORKSPACE_INCOMPLETE",
                "worktree creation is incomplete; inspect the retained binding before manual recovery",
            ));
        }
        validate_live(&receipt)?;
        private_child(&dir, "proxy", false)?
            .ok_or_else(|| fail("WORKSPACE_INCOMPLETE", "private proxy directory is missing"))?;
        Ok(WorktreeBinding {
            newly_created: false,
            proxy_path: binding_path(&receipt).join("proxy"),
            storage: Storage {
                directory: dir,
                lock,
                path: binding_path(&receipt),
                expected_hash: Some(crate::hash(&bytes)),
            },
            receipt,
        })
    }

    pub(super) fn inspect(
        source: &Path,
        project: &str,
        selector: &str,
        work: &str,
        managed: Option<&Path>,
    ) -> Result<WorkspacePlan> {
        let expected = expected(source, project, selector, work, managed)?;
        if let Some(store) = store(&expected.common_dir, false)? {
            if let Some(dir) = private_child(&store, work, false)? {
                let receipt = parse(&read_receipt(&dir)?, &expected)?;
                if receipt.status == BindingStatus::Ready {
                    validate_live(&receipt)?;
                    private_child(&dir, "proxy", false)?.ok_or_else(|| {
                        fail("WORKSPACE_INCOMPLETE", "private proxy directory is missing")
                    })?;
                }
                return Ok(receipt.plan(true));
            }
        }
        collision(&expected)?;
        Ok(expected.plan(false))
    }

    pub(super) fn acquire(
        source: &Path,
        project: &str,
        selector: &str,
        work: &str,
        managed: Option<&Path>,
        create: bool,
    ) -> Result<WorktreeBinding> {
        let mut receipt = expected(source, project, selector, work, managed)?;
        if let Some(store) = store(&receipt.common_dir, false)? {
            if let Some(dir) = private_child(&store, work, false)? {
                return existing(dir, &receipt);
            }
        }
        if !create {
            return Err(fail(
                "WORKSPACE_NOT_BOUND",
                "no existing worktree binding for this work ID",
            ));
        }
        let store = store(&receipt.common_dir, true)?.expect("created store");
        let _creation_lock = lock(&store, "creation.lock", true, true)?;
        if let Some(dir) = private_child(&store, work, false)? {
            return existing(dir, &receipt);
        }
        collision(&receipt)?;
        let dir = private_child(&store, work, true)?.expect("created binding directory");
        let owner = lock(&dir, "owner.lock", true, false)?;
        let mut storage = Storage {
            directory: dir,
            lock: owner,
            path: binding_path(&receipt),
            expected_hash: None,
        };
        receipt.status = BindingStatus::Preparing;
        storage.write(&receipt)?;
        store.sync_all().map_err(io_error)?;
        let result = (|| {
            private_child(&storage.directory, "proxy", true)?;
            directory(receipt.root.parent().expect("absolute worktree path"), true)?;
            // A final collision check after making parents still never adopts an existing path.
            collision(&receipt)?;
            git(
                source,
                &[
                    OsStr::new("worktree"),
                    OsStr::new("add"),
                    OsStr::new("--no-checkout"),
                    OsStr::new("--no-track"),
                    OsStr::new("-b"),
                    OsStr::new(&receipt.branch),
                    OsStr::new("--"),
                    receipt.root.as_os_str(),
                    OsStr::new(&receipt.base_commit),
                ],
                false,
            )?;
            // Register first, then populate the new index/worktree without force.
            // Git's worktree-add cleanup must not erase a failed checkout's files.
            git(
                &receipt.root,
                &[OsStr::new("read-tree"), OsStr::new(&receipt.base_commit)],
                false,
            )?;
            git(
                &receipt.root,
                &[OsStr::new("checkout-index"), OsStr::new("--all")],
                false,
            )?;
            receipt.root_identity = Some(identity(
                &required_dir(&receipt.root)?.metadata().map_err(io_error)?,
            ));
            let git_dir = PathBuf::from(git_text(
                &receipt.root,
                &["rev-parse", "--absolute-git-dir"],
            )?);
            receipt.git_identity = Some(identity(
                &required_dir(&git_dir)?.metadata().map_err(io_error)?,
            ));
            receipt.git_dir = Some(git_dir);
            receipt.status = BindingStatus::Ready;
            validate_live(&receipt)?;
            receipt.creation_proof = Some(recovery::CreationProof {
                schema_version: 1,
                root_identity: receipt.root_identity.expect("validated root identity"),
                git_dir: receipt.git_dir.clone().expect("validated Git directory"),
                git_identity: receipt.git_identity.expect("validated Git identity"),
            });
            let mut prepared = receipt.clone();
            prepared.status = BindingStatus::Preparing;
            prepared.root_identity = None;
            prepared.git_dir = None;
            prepared.git_identity = None;
            storage.write(&prepared)?;
            storage.write(&receipt)
        })();
        if let Err(error) = result {
            receipt.status = BindingStatus::Failed;
            receipt.root_identity = None;
            receipt.git_dir = None;
            receipt.git_identity = None;
            receipt.last_error = Some(error.code.into());
            let _ = storage.write(&receipt); // Earlier preparing receipt remains if persistence fails.
            return Err(error);
        }
        Ok(WorktreeBinding {
            newly_created: true,
            proxy_path: binding_path(&receipt).join("proxy"),
            receipt,
            storage,
        })
    }
}
