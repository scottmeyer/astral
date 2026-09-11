//! Read-only, explicit-root observations. These never establish runtime availability.
use super::*;

pub const MAX_INVENTORY_ENTRIES: usize = 1024;
pub const MAX_INVENTORY_PAGE: usize = 64;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchOwnership {
    Available,
    Busy,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LaunchContinuation {
    Unknown,
    Owned,
    InitialOutcomeUnknown,
    RecordedContinuationCandidate,
}

#[derive(Debug, Serialize)]
pub struct LaunchReceiptSummary {
    pub workspace: PathBuf,
    pub project_id: String,
    pub selector: String,
    pub work_id: Option<String>,
    pub thread_id: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub selection_digest: String,
    pub bundle_manifest_sha256: String,
    pub status: &'static str,
    pub exit_code: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LaunchObservation {
    pub launch_id: String,
    pub receipt: Option<LaunchReceiptSummary>,
    pub receipt_sha256: Option<String>,
    pub ownership: LaunchOwnership,
    pub continuation: LaunchContinuation,
    pub issues: Vec<Error>,
}

#[derive(Debug, Serialize)]
pub struct LaunchInventory {
    pub schema_version: u32,
    pub state_root: PathBuf,
    pub workspace: PathBuf,
    pub project_id: String,
    pub total_entries: usize,
    pub excluded_foreign_launches: usize,
    pub unrecognized_entries: usize,
    pub total_observations: usize,
    pub offset: usize,
    pub next_offset: Option<usize>,
    pub launches: Vec<LaunchObservation>,
    pub issues: Vec<Error>,
    pub observation_scope: &'static str,
}

fn unknown(id: &str, error: Error) -> LaunchObservation {
    LaunchObservation {
        launch_id: id.into(),
        receipt: None,
        receipt_sha256: None,
        ownership: LaunchOwnership::Unknown,
        continuation: LaunchContinuation::Unknown,
        issues: vec![error],
    }
}

fn workspace_identity(root: &Path, project: &str) -> Result<String> {
    let expected = identity(
        root,
        project,
        "project-context",
        None,
        &"0".repeat(64),
        &"0".repeat(64),
    )?;
    Ok(expected.workspace)
}

impl LaunchState {
    /// Does not retain ownership. A later continuation must acquire and revalidate.
    pub fn observe_in(
        state_root: &Path,
        id: &str,
        workspace: &Path,
        project_id: &str,
    ) -> Result<LaunchObservation> {
        if !hex(id, 32) {
            return Err(fail(
                "LAUNCH_STATE_ID",
                "launch ID must contain 32 lowercase hexadecimal characters",
            ));
        }
        let workspace = workspace_identity(workspace, project_id)?;
        #[cfg(unix)]
        {
            let root = super::unix::state_root(state_root, &workspace, false)?;
            Ok(
                unix::observe(&root, state_root, id, &workspace, project_id).unwrap_or_else(|| {
                    unknown(
                        id,
                        fail(
                            "LAUNCH_STATE_MISMATCH",
                            "launch receipt belongs to a different workspace or project",
                        ),
                    )
                }),
            )
        }
        #[cfg(not(unix))]
        {
            let _ = (state_root, workspace);
            Err(fail(
                "LAUNCH_STATE_UNSUPPORTED_PLATFORM",
                "private launch observations currently require Unix",
            ))
        }
    }

    /// Enumerates only the explicitly supplied root. Valid foreign records are omitted.
    pub fn inventory_in(
        state_root: &Path,
        workspace: &Path,
        project_id: &str,
        offset: usize,
        limit: usize,
    ) -> Result<LaunchInventory> {
        if state_root.to_str().is_none() {
            return Err(fail(
                "LAUNCH_STATE_PATH",
                "launch inventory root requires a UTF-8 path for its metadata report",
            ));
        }
        if limit == 0 || limit > MAX_INVENTORY_PAGE {
            return Err(fail(
                "LAUNCH_STATE_PAGE",
                "launch inventory limit must be between 1 and 64",
            ));
        }
        let workspace = workspace_identity(workspace, project_id)?;
        #[cfg(unix)]
        {
            unix::inventory(state_root, &workspace, project_id, offset, limit)
        }
        #[cfg(not(unix))]
        {
            let _ = (state_root, workspace, offset);
            Err(fail(
                "LAUNCH_STATE_UNSUPPORTED_PLATFORM",
                "private launch observations currently require Unix",
            ))
        }
    }
}

#[cfg(unix)]
mod unix {
    use super::super::unix::{directory_at, secure, state_root};
    use super::*;
    use std::collections::BTreeSet;
    use std::ffi::{CStr, OsStr, OsString};
    use std::fs::{File, Metadata};
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::MetadataExt;

    fn io() -> Error {
        fail("LAUNCH_STATE_IO", "cannot read launch state metadata")
    }
    fn changed() -> Error {
        fail(
            "LAUNCH_STATE_CHANGED",
            "launch state changed during observation; retry the observation",
        )
    }
    fn metadata(file: &File) -> Result<Metadata> {
        file.metadata().map_err(|_| io())
    }
    fn same_file(a: &File, b: &File) -> Result<bool> {
        let a = metadata(a)?;
        let b = metadata(b)?;
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }
    fn stamp(m: &Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
        (
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    }
    fn readonly(parent: &File, name: &CStr) -> Result<File> {
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(fail(
                "LAUNCH_STATE_UNAVAILABLE",
                "launch state entry is missing, unsafe, or unavailable",
            ));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        secure(&metadata(&file)?, false)?;
        Ok(file)
    }
    struct Snapshot {
        file: File,
        stamp: (u64, u64, u64, i64, i64, i64, i64),
        bytes: Vec<u8>,
    }
    impl Snapshot {
        fn read(directory: &File) -> Result<Self> {
            let mut file = readonly(directory, c"receipt.json")?;
            let before = metadata(&file)?;
            if before.len() > MAX_RECEIPT_BYTES as u64 {
                return Err(fail(
                    "LAUNCH_STATE_LIMIT",
                    "launch receipt exceeds its byte limit",
                ));
            }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(MAX_RECEIPT_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| io())?;
            let after = metadata(&file)?;
            secure(&after, false)?;
            if bytes.len() > MAX_RECEIPT_BYTES {
                return Err(fail(
                    "LAUNCH_STATE_LIMIT",
                    "launch receipt exceeds its byte limit",
                ));
            }
            if stamp(&before) != stamp(&after) || bytes.len() as u64 != after.len() {
                return Err(changed());
            }
            Ok(Self {
                file,
                stamp: stamp(&after),
                bytes,
            })
        }
        fn unchanged(&self, directory: &File) -> Result<()> {
            let now = Self::read(directory)?;
            if self.stamp != now.stamp
                || self.bytes != now.bytes
                || !same_file(&self.file, &now.file)?
            {
                return Err(changed());
            }
            Ok(())
        }
    }
    struct Probe {
        file: File,
        held: bool,
    }
    impl Probe {
        fn acquire(file: File) -> Result<(Self, LaunchOwnership)> {
            match fs2::FileExt::try_lock_exclusive(&file) {
                Ok(()) => Ok((Self { file, held: true }, LaunchOwnership::Available)),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    Ok((Self { file, held: false }, LaunchOwnership::Busy))
                }
                Err(_) => Err(fail("LAUNCH_STATE_LOCK", "cannot observe launch ownership")),
            }
        }
        fn release(&mut self) -> Result<()> {
            if self.held {
                fs2::FileExt::unlock(&self.file).map_err(|_| {
                    fail(
                        "LAUNCH_STATE_LOCK",
                        "cannot release launch observation lock",
                    )
                })?;
                self.held = false;
            }
            Ok(())
        }
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            if self.held {
                let _ = fs2::FileExt::unlock(&self.file);
            }
        }
    }

    pub(super) fn observe(
        root: &File,
        path: &Path,
        id: &str,
        workspace: &str,
        project: &str,
    ) -> Option<LaunchObservation> {
        match observe_checked(root, path, id, workspace, project) {
            Ok(result) => result,
            Err(error) => Some(unknown(id, error)),
        }
    }

    fn observe_checked(
        root: &File,
        path: &Path,
        id: &str,
        workspace: &str,
        project: &str,
    ) -> Result<Option<LaunchObservation>> {
        let directory = directory_at(root, OsStr::new(id), false)?;
        let snapshot = Snapshot::read(&directory)?;
        let receipt: Receipt = serde_json::from_slice(&snapshot.bytes).map_err(|_| {
            fail(
                "LAUNCH_STATE_INVALID",
                "launch receipt is not valid strict JSON metadata",
            )
        })?;
        validate_receipt_metadata(&receipt)?;
        if receipt.id != id {
            return Err(fail(
                "LAUNCH_STATE_INVALID",
                "launch receipt ID differs from its directory",
            ));
        }
        let foreign = receipt.workspace != workspace || receipt.project_id != project;
        // Even excluded records must be a stable, validated observation. Never use
        // a foreign receipt's workspace path for filesystem access.
        if foreign {
            snapshot.unchanged(&directory)?;
            let now_root = state_root(path, workspace, false)?;
            let now_dir = directory_at(&now_root, OsStr::new(id), false)?;
            if !same_file(root, &now_root)? || !same_file(&directory, &now_dir)? {
                return Err(changed());
            }
            return Ok(None);
        }
        let lock = readonly(&directory, c"owner.lock")?;
        let proxy = directory_at(&directory, OsStr::new("proxy"), false)?;
        let (mut probe, ownership) = Probe::acquire(lock)?;
        snapshot.unchanged(&directory)?;
        let now_root = state_root(path, workspace, false)?;
        let now_dir = directory_at(&now_root, OsStr::new(id), false)?;
        let now_lock = readonly(&now_dir, c"owner.lock")?;
        let now_proxy = directory_at(&now_dir, OsStr::new("proxy"), false)?;
        if !same_file(root, &now_root)?
            || !same_file(&directory, &now_dir)?
            || !same_file(&probe.file, &now_lock)?
            || !same_file(&proxy, &now_proxy)?
        {
            return Err(changed());
        }
        snapshot.unchanged(&now_dir)?;
        probe.release()?;
        let continuation = match ownership {
            LaunchOwnership::Busy => LaunchContinuation::Owned,
            LaunchOwnership::Available if receipt.thread_id.is_some() => {
                LaunchContinuation::RecordedContinuationCandidate
            }
            LaunchOwnership::Available => LaunchContinuation::InitialOutcomeUnknown,
            LaunchOwnership::Unknown => LaunchContinuation::Unknown,
        };
        let status = match receipt.status {
            Status::Preparing => "preparing",
            Status::Staged => "staged",
            Status::Finished => "finished",
            Status::Failed => "failed",
        };
        Ok(Some(LaunchObservation {
            launch_id: id.into(),
            receipt_sha256: Some(crate::hash(&snapshot.bytes)),
            ownership,
            continuation,
            issues: Vec::new(),
            receipt: Some(LaunchReceiptSummary {
                workspace: receipt.workspace.into(),
                project_id: receipt.project_id,
                selector: receipt.selector,
                work_id: receipt.work_id,
                thread_id: receipt.thread_id,
                model: receipt.model,
                provider: receipt.provider,
                selection_digest: receipt.selection_digest,
                bundle_manifest_sha256: receipt.bundle_manifest_sha256,
                status,
                exit_code: receipt.exit_code,
                last_error: receipt.last_error,
            }),
        }))
    }

    fn names(root: &File) -> Result<BTreeSet<OsString>> {
        let fd = unsafe {
            libc::openat(
                root.as_raw_fd(),
                c".".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            )
        };
        if fd < 0 {
            return Err(io());
        }
        let raw = unsafe { libc::fdopendir(fd) };
        if raw.is_null() {
            unsafe {
                libc::close(fd);
            }
            return Err(io());
        }
        struct Directory(*mut libc::DIR);
        impl Drop for Directory {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let directory = Directory(raw);
        let mut result = BTreeSet::new();
        loop {
            errno::set_errno(errno::Errno(0));
            let entry = unsafe { libc::readdir(directory.0) };
            if entry.is_null() {
                if errno::errno().0 != 0 {
                    return Err(io());
                }
                break;
            }
            let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if bytes == b"." || bytes == b".." {
                continue;
            }
            if result.len() >= MAX_INVENTORY_ENTRIES {
                return Err(fail(
                    "LAUNCH_STATE_LIMIT",
                    "launch inventory exceeds 1024 directory entries",
                ));
            }
            if !result.insert(OsString::from_vec(bytes.to_vec())) {
                return Err(changed());
            }
        }
        Ok(result)
    }

    pub(super) fn inventory(
        path: &Path,
        workspace: &str,
        project: &str,
        offset: usize,
        limit: usize,
    ) -> Result<LaunchInventory> {
        let root = state_root(path, workspace, false)?;
        let before = metadata(&root)?;
        let entries = names(&root)?;
        let mut observations = Vec::new();
        let mut foreign = 0;
        let mut unrecognized = 0;
        for entry in &entries {
            let Some(id) = entry.to_str().filter(|id| hex(id, 32)) else {
                unrecognized += 1;
                continue;
            };
            match observe(&root, path, id, workspace, project) {
                Some(observation) => observations.push(observation),
                None => foreign += 1,
            }
        }
        let now = state_root(path, workspace, false)?;
        if !same_file(&root, &now)?
            || stamp(&before) != stamp(&metadata(&now)?)
            || names(&now)? != entries
        {
            return Err(changed());
        }
        let total = observations.len();
        let launches: Vec<_> = observations.into_iter().skip(offset).take(limit).collect();
        let next = offset.saturating_add(launches.len());
        let issues = if unrecognized == 0 {
            Vec::new()
        } else {
            vec![fail(
                "LAUNCH_STATE_UNKNOWN_ENTRY",
                "unrecognized entries were counted without opening or displaying their names",
            )]
        };
        Ok(LaunchInventory {
            schema_version: 1,
            state_root: path.into(),
            workspace: workspace.into(),
            project_id: project.into(),
            total_entries: entries.len(),
            excluded_foreign_launches: foreign,
            unrecognized_entries: unrecognized,
            total_observations: total,
            offset,
            next_offset: (next < total).then_some(next),
            launches,
            issues,
            observation_scope: "Bounded explicit-root receipt observations only. Ownership is a momentary advisory probe; a recorded thread is not proof of runtime availability or context compatibility. No runtime calls or repairs were performed.",
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn snapshot_rejects_same_bytes_replacement_and_in_place_mutation() {
            for atomic in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let root = temp.path().canonicalize().unwrap();
                let path = root.join("receipt.json");
                std::fs::write(&path, b"original bytes").unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
                let directory = File::open(&root).unwrap();
                let before = Snapshot::read(&directory).unwrap();
                if atomic {
                    let replacement = root.join("replacement.json");
                    std::fs::write(&replacement, b"original bytes").unwrap();
                    std::fs::set_permissions(&replacement, std::fs::Permissions::from_mode(0o600))
                        .unwrap();
                    std::fs::rename(&replacement, &path).unwrap();
                } else {
                    std::fs::write(&path, b"changed! bytes").unwrap();
                }
                assert_eq!(
                    before.unchanged(&directory).unwrap_err().code,
                    "LAUNCH_STATE_CHANGED"
                );
            }
        }
    }
}
