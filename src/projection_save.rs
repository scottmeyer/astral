//! Atomic named projection publication over immutable native bundle files.
//!
//! Cooperating writers lock the `.astral` directory inode. Reads and writes are
//! descriptor-relative and reject links, special files, and foreign ownership.
//! A bounded private preview is validated before publication; failed previews
//! and old bundles are retained. Final comparison detects observed external
//! edits, but cannot exclude an uncooperative writer in the comparison/rename
//! window. File contents are synced; directory sync is best effort, so the
//! guarantee is atomic visibility rather than power-loss durability.

use crate::native_bundle::NativeBundle;
use crate::project::{Error, NativeBundleReference, ProjectionManifest, Result, SourceHandle};
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct PublicationReceipt {
    pub projection: String,
    pub projection_manifest: SourceHandle,
    pub bundle_manifest: SourceHandle,
    pub payload: SourceHandle,
}

fn fail(code: &'static str, message: &'static str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn handle(path: String, bytes: &[u8]) -> SourceHandle {
    SourceHandle {
        path,
        sha256: crate::hash(bytes),
        bytes: bytes.len(),
        record_id: None,
    }
}

/// Publish a new named projection or atomically advance an existing reference.
/// Existing subsystem scope, handoff path/bytes, schema notes and provenance are
/// retained. `handoff_text` applies only to a new projection.
pub fn publish(
    root: &Path,
    name: &str,
    subsystem_ids: &[String],
    bundle: &NativeBundle,
    handoff_text: Option<&str>,
) -> Result<PublicationReceipt> {
    #[cfg(unix)]
    {
        unix::publish(root, name, subsystem_ids, bundle, handoff_text)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, name, subsystem_ids, bundle, handoff_text);
        Err(fail(
            "UNSUPPORTED_PLATFORM",
            "native projection publication requires Unix",
        ))
    }
}

/// Carry the entire validated work JSONL into a newly acquired worktree. Only
/// this configured file is copied, preserving its exact bytes and dependencies.
/// Both source and target snapshots are compared again before atomic replacement.
pub fn carry_work_file(source: &Path, target: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        unix::carry_work_file(source, target)
    }
    #[cfg(not(unix))]
    {
        let _ = (source, target);
        Err(fail(
            "UNSUPPORTED_PLATFORM",
            "work-file carry requires Unix",
        ))
    }
}

/// Cheap capture preflight: validate the current target, scope and same-device
/// private staging location before asking the runtime to compact anything.
pub fn validate_target(root: &Path, name: &str, subsystem_ids: &[String]) -> Result<()> {
    #[cfg(unix)]
    {
        unix::validate_target(root, name, subsystem_ids)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, name, subsystem_ids);
        Err(fail(
            "UNSUPPORTED_PLATFORM",
            "native projection publication requires Unix",
        ))
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::project::{Limits, Project, ProjectManifest};
    use std::collections::{BTreeMap, BTreeSet};
    use std::ffi::CString;
    use std::fs::{File, Metadata};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::{Component, PathBuf};

    const STAGING: &str = "astral/publication-staging";
    const MAX_HANDOFF: usize = 32_768;

    fn relative(path: &str) -> Result<()> {
        if path.is_empty()
            || path.len() > 4096
            || path.contains(['\\', ':', '\0'])
            || path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(fail(
                "UNSAFE_PATH",
                "expected a confined relative publication path",
            ));
        }
        Ok(())
    }

    fn identifier(value: &str) -> Result<()> {
        if value.is_empty()
            || value.len() > 128
            || value == "."
            || value == ".."
            || !value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(fail(
                "INVALID_ID",
                "invalid projection or subsystem identifier",
            ));
        }
        Ok(())
    }

    fn cstring(value: &str) -> Result<CString> {
        CString::new(value).map_err(|_| fail("UNSAFE_PATH", "publication path contains a NUL byte"))
    }

    fn secure(metadata: &Metadata, directory: bool) -> Result<()> {
        if metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o022 != 0
            || if directory {
                !metadata.is_dir()
            } else {
                !metadata.is_file() || metadata.nlink() != 1
            }
        {
            return Err(fail(
                "PUBLICATION_INSECURE",
                "publication entries must be owned, non-writable by others, and free of hardlinks or special files",
            ));
        }
        Ok(())
    }

    fn absolute_directory(path: &Path) -> Result<File> {
        let mut current = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")
            .map_err(|_| {
                fail(
                    "PUBLICATION_PATH_UNAVAILABLE",
                    "filesystem root is unavailable",
                )
            })?;
        for component in path.components() {
            match component {
                Component::RootDir => {}
                Component::Normal(name) => {
                    let name = cstring(name.to_str().ok_or_else(|| {
                        fail("UNSAFE_PATH", "Git common directory must be UTF-8")
                    })?)?;
                    let fd = unsafe {
                        libc::openat(
                            current.as_raw_fd(),
                            name.as_ptr(),
                            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                        )
                    };
                    if fd < 0 {
                        return Err(fail(
                            "PUBLICATION_PATH_UNAVAILABLE",
                            "Git common directory contains a link or unavailable component",
                        ));
                    }
                    current = unsafe { File::from_raw_fd(fd) };
                }
                _ => {
                    return Err(fail(
                        "UNSAFE_PATH",
                        "Git common directory must be an absolute path without dot components",
                    ));
                }
            }
        }
        Ok(current)
    }

    fn open(parent: &File, component: &str, directory: bool) -> Result<File> {
        let component = cstring(component)?;
        let flags = libc::O_RDONLY
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | libc::O_CLOEXEC
            | if directory { libc::O_DIRECTORY } else { 0 };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), component.as_ptr(), flags) };
        if fd < 0 {
            return Err(fail(
                "PUBLICATION_PATH_UNAVAILABLE",
                "publication entry is missing, linked, or unavailable",
            ));
        }
        let file = unsafe { File::from_raw_fd(fd) };
        secure(
            &file
                .metadata()
                .map_err(|_| fail("PUBLICATION_IO", "cannot inspect publication entry"))?,
            directory,
        )?;
        Ok(file)
    }

    fn exists(parent: &File, component: &str) -> Result<bool> {
        let component = cstring(component)?;
        let mut metadata = std::mem::MaybeUninit::<libc::stat>::uninit();
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                component.as_ptr(),
                metadata.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(fail(
                "PUBLICATION_IO",
                "cannot inspect publication destination",
            ))
        }
    }

    fn directory(parent: &File, path: &str, create: bool) -> Result<File> {
        relative(path)?;
        let mut current = parent
            .try_clone()
            .map_err(|_| fail("PUBLICATION_IO", "cannot retain publication directory"))?;
        for component in path.split('/') {
            if create {
                let name = cstring(component)?;
                if unsafe { libc::mkdirat(current.as_raw_fd(), name.as_ptr(), 0o700) } != 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(fail(
                        "PUBLICATION_IO",
                        "cannot create publication directory",
                    ));
                }
            }
            current = open(&current, component, true)?;
        }
        Ok(current)
    }

    fn parent(root: &File, path: &str, create: bool) -> Result<(File, String)> {
        relative(path)?;
        match path.rsplit_once('/') {
            Some((path, name)) => Ok((directory(root, path, create)?, name.to_owned())),
            None => Ok((
                root.try_clone()
                    .map_err(|_| fail("PUBLICATION_IO", "cannot retain publication directory"))?,
                path.to_owned(),
            )),
        }
    }

    fn read(root: &File, path: &str, limit: usize) -> Result<Vec<u8>> {
        let (parent, name) = parent(root, path, false)?;
        let mut file = open(&parent, &name, false)?;
        if file
            .metadata()
            .map_err(|_| fail("PUBLICATION_IO", "cannot inspect publication file"))?
            .len()
            > limit as u64
        {
            return Err(fail("LIMIT_EXCEEDED", "publication file byte limit"));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| fail("PUBLICATION_IO", "cannot read publication file"))?;
        if bytes.len() > limit {
            return Err(fail("LIMIT_EXCEEDED", "publication file byte limit"));
        }
        Ok(bytes)
    }

    fn write_new(root: &File, path: &str, bytes: &[u8]) -> Result<()> {
        let (parent, name) = parent(root, path, true)?;
        let name = cstring(&name)?;
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(fail(
                "PUBLICATION_CONFLICT",
                "publication staging file already exists or is unavailable",
            ));
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| fail("PUBLICATION_IO", "cannot persist publication staging file"))?;
        let _ = parent.sync_all();
        Ok(())
    }

    fn rename(
        source: &File,
        source_path: &str,
        target: &File,
        target_path: &str,
        exclusive: bool,
    ) -> Result<()> {
        let (source_parent, source_name) = parent(source, source_path, false)?;
        let (target_parent, target_name) = parent(target, target_path, false)?;
        let source_name = cstring(&source_name)?;
        let target_name = cstring(&target_name)?;
        let result = if exclusive {
            #[cfg(target_os = "linux")]
            {
                unsafe {
                    libc::renameat2(
                        source_parent.as_raw_fd(),
                        source_name.as_ptr(),
                        target_parent.as_raw_fd(),
                        target_name.as_ptr(),
                        libc::RENAME_NOREPLACE,
                    )
                }
            }
            #[cfg(target_os = "macos")]
            {
                unsafe {
                    libc::renameatx_np(
                        source_parent.as_raw_fd(),
                        source_name.as_ptr(),
                        target_parent.as_raw_fd(),
                        target_name.as_ptr(),
                        libc::RENAME_EXCL,
                    )
                }
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                return Err(fail(
                    "UNSUPPORTED_PLATFORM",
                    "exclusive directory publication requires Linux or macOS",
                ));
            }
        } else {
            unsafe {
                libc::renameat(
                    source_parent.as_raw_fd(),
                    source_name.as_ptr(),
                    target_parent.as_raw_fd(),
                    target_name.as_ptr(),
                )
            }
        };
        if result != 0 {
            return Err(fail(
                "PUBLICATION_CONFLICT",
                "publication destination changed or atomic rename failed",
            ));
        }
        let _ = source_parent.sync_all();
        let _ = target_parent.sync_all();
        Ok(())
    }

    struct Repository {
        path: PathBuf,
        root: File,
        astral: File,
    }

    impl Drop for Repository {
        fn drop(&mut self) {
            // Explicit unlock avoids a concurrently forked child briefly
            // retaining the open-file-description lock until its exec closes FDs.
            let _ = fs2::FileExt::unlock(&self.astral);
        }
    }

    impl Repository {
        fn lock(root: &Path) -> Result<Self> {
            let path = root
                .canonicalize()
                .map_err(|_| fail("INVALID_ROOT", "publication workspace is unavailable"))?;
            let root = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)
                .map_err(|_| fail("INVALID_ROOT", "publication workspace must be a directory"))?;
            secure(
                &root
                    .metadata()
                    .map_err(|_| fail("PUBLICATION_IO", "cannot inspect workspace"))?,
                true,
            )?;
            let astral = open(&root, ".astral", true)?;
            fs2::FileExt::try_lock_exclusive(&astral)
                .map_err(|_| fail("PUBLICATION_BUSY", "another writer owns the project index"))?;
            Ok(Self { path, root, astral })
        }

        fn unchanged_root(&self) -> Result<()> {
            let current = open(&self.root, ".astral", true)?
                .metadata()
                .map_err(|_| fail("PUBLICATION_IO", "cannot inspect project index"))?;
            let original = self
                .astral
                .metadata()
                .map_err(|_| fail("PUBLICATION_IO", "cannot inspect locked project index"))?;
            if (current.dev(), current.ino()) != (original.dev(), original.ino()) {
                return Err(fail(
                    "PUBLICATION_STALE",
                    "project index directory changed during publication",
                ));
            }
            Ok(())
        }
    }

    struct Snapshot {
        manifest: ProjectManifest,
        files: BTreeMap<String, Vec<u8>>,
        contexts: serde_json::Value,
    }

    impl Snapshot {
        fn load(repository: &Repository) -> Result<Self> {
            let project = Project::load(&repository.path)?;
            let manifest_bytes = read(
                &repository.root,
                ".astral/project.toml",
                Limits::default().file_bytes,
            )?;
            let manifest: ProjectManifest = toml::from_str(
                std::str::from_utf8(&manifest_bytes)
                    .map_err(|_| fail("INVALID_UTF8", "project manifest must be UTF-8"))?,
            )
            .map_err(|_| fail("INVALID_MANIFEST", "project manifest is invalid"))?;
            relative(&manifest.projections)?;
            relative(&manifest.work_items)?;
            let contexts = project.list()?["contexts"].clone();
            let mut expected = BTreeMap::new();
            for context in contexts.as_array().expect("Project list contains contexts") {
                let inspection = project.inspect(
                    context["selector"].as_str().expect("selector is a string"),
                    None,
                )?;
                for source in inspection["sources"]
                    .as_array()
                    .expect("inspection sources are an array")
                {
                    expected.insert(
                        source["path"]
                            .as_str()
                            .expect("source path is a string")
                            .to_owned(),
                        source["sha256"]
                            .as_str()
                            .expect("source hash is a string")
                            .to_owned(),
                    );
                }
            }
            let mut files = BTreeMap::new();
            files.insert(".astral/project.toml".into(), manifest_bytes);
            let work_path = format!(".astral/{}", manifest.work_items);
            files.insert(
                work_path.clone(),
                read(&repository.root, &work_path, Limits::default().file_bytes)?,
            );
            for (path, hash) in expected {
                let bytes = read(
                    &repository.root,
                    &path,
                    Limits::default().native_payload_bytes,
                )?;
                if crate::hash(&bytes) != hash {
                    return Err(fail(
                        "PUBLICATION_STALE",
                        "declared source changed during snapshot",
                    ));
                }
                files.insert(path, bytes);
            }
            // A project can have no contexts: core files still belong in preview.
            for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
                let path = format!(".astral/{}/{name}", manifest.core);
                if !files.contains_key(&path) {
                    files.insert(
                        path.clone(),
                        read(&repository.root, &path, Limits::default().file_bytes)?,
                    );
                }
            }
            budget(&files)?;
            Ok(Self {
                manifest,
                files,
                contexts,
            })
        }

        fn unchanged(&self, repository: &Repository) -> Result<()> {
            repository.unchanged_root()?;
            for (path, bytes) in &self.files {
                if read(
                    &repository.root,
                    path,
                    Limits::default().native_payload_bytes,
                )? != *bytes
                {
                    return Err(fail(
                        "PUBLICATION_STALE",
                        "project source changed before publication",
                    ));
                }
            }
            if Project::load(&repository.path)?.list()?["contexts"] != self.contexts {
                return Err(fail(
                    "PUBLICATION_STALE",
                    "project context registry changed before publication",
                ));
            }
            Ok(())
        }
    }

    fn budget(files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
        let limits = Limits::default();
        if files.len() > limits.files
            || files.values().map(Vec::len).sum::<usize>() > limits.total_bytes
        {
            return Err(fail(
                "LIMIT_EXCEEDED",
                "publication preview exceeds project aggregate limits",
            ));
        }
        Ok(())
    }

    fn same_device(left: &File, right: &File) -> Result<()> {
        let left = left
            .metadata()
            .map_err(|_| fail("PUBLICATION_IO", "cannot inspect publication filesystem"))?;
        let right = right
            .metadata()
            .map_err(|_| fail("PUBLICATION_IO", "cannot inspect publication filesystem"))?;
        if left.dev() != right.dev() {
            return Err(fail(
                "PUBLICATION_CROSS_DEVICE",
                "atomic publication requires staging and destination on one filesystem",
            ));
        }
        Ok(())
    }

    fn staging(repository: &Repository) -> Result<(File, PathBuf)> {
        // Git resolves a symlinked .git directory before printing its common
        // path; reject that workspace entry before consulting the resolver.
        let git_name = cstring(".git")?;
        let git_fd = unsafe {
            libc::openat(
                repository.root.as_raw_fd(),
                git_name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if git_fd < 0 {
            return Err(fail(
                "PUBLICATION_PATH_UNAVAILABLE",
                "workspace Git metadata is linked or unavailable",
            ));
        }
        let git_entry = unsafe { File::from_raw_fd(git_fd) };
        let git_metadata = git_entry
            .metadata()
            .map_err(|_| fail("PUBLICATION_IO", "cannot inspect workspace Git metadata"))?;
        secure(&git_metadata, git_metadata.is_dir())?;
        let bytes = crate::workspace::git_read(
            &repository.path,
            &[
                "rev-parse".into(),
                "--path-format=absolute".into(),
                "--git-common-dir".into(),
            ],
        )?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| {
                fail(
                    "PUBLICATION_LAYOUT_CONFLICT",
                    "Git common directory must be UTF-8",
                )
            })?
            .trim_end_matches('\n');
        if text.len() > 4096 || text.contains(['\n', '\r', '\0']) || !Path::new(text).is_absolute()
        {
            return Err(fail(
                "PUBLICATION_LAYOUT_CONFLICT",
                "Git returned an invalid common directory",
            ));
        }
        let path = PathBuf::from(text);
        let common = absolute_directory(&path)?;
        let metadata = common
            .metadata()
            .map_err(|_| fail("PUBLICATION_IO", "cannot inspect Git common directory"))?;
        secure(&metadata, true)?;
        if metadata.dev()
            != repository
                .astral
                .metadata()
                .map_err(|_| fail("PUBLICATION_IO", "cannot inspect project filesystem"))?
                .dev()
        {
            return Err(fail(
                "PUBLICATION_CROSS_DEVICE",
                "publication currently requires worktree and Git common directory on one filesystem",
            ));
        }
        let staging = directory(&common, STAGING, true)?;
        same_device(&staging, &repository.astral)?;
        if staging
            .metadata()
            .map_err(|_| fail("PUBLICATION_IO", "cannot inspect staging directory"))?
            .mode()
            & 0o777
            != 0o700
        {
            return Err(fail(
                "PUBLICATION_INSECURE",
                "publication staging must be private",
            ));
        }
        Ok((staging, path.join(STAGING)))
    }

    fn preview(
        repository: &Repository,
        files: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(File, PathBuf)> {
        budget(files)?;
        let (staging, staging_path) = staging(repository)?;
        let mut random = [0u8; 16];
        getrandom::fill(&mut random)
            .map_err(|_| fail("PUBLICATION_IO", "cannot allocate publication identifier"))?;
        let name = crate::hash(&random);
        let c_name = cstring(&name)?;
        if unsafe { libc::mkdirat(staging.as_raw_fd(), c_name.as_ptr(), 0o700) } != 0 {
            return Err(fail(
                "PUBLICATION_CONFLICT",
                "publication staging identifier is unavailable",
            ));
        }
        let directory = open(&staging, &name, true)?;
        for (path, bytes) in files {
            write_new(&directory, path, bytes)?;
        }
        let path = staging_path.join(name);
        Project::load(&path)?.validate()?;
        Ok((directory, path))
    }

    pub(super) fn validate_target(root: &Path, name: &str, subsystem_ids: &[String]) -> Result<()> {
        identifier(name)?;
        let mut scope = BTreeSet::new();
        for id in subsystem_ids {
            identifier(id)?;
            if !scope.insert(id.clone()) {
                return Err(fail(
                    "DUPLICATE_REFERENCE",
                    "projection subsystem scope contains duplicates",
                ));
            }
        }
        if scope.len() > Limits::default().entries {
            return Err(fail("LIMIT_EXCEEDED", "projection subsystem limit"));
        }
        let repository = Repository::lock(root)?;
        let snapshot = Snapshot::load(&repository)?;
        if scope
            .iter()
            .any(|id| !snapshot.manifest.subsystems.contains_key(id))
        {
            return Err(fail(
                "MISSING_REFERENCE",
                "saved projection names an unknown subsystem",
            ));
        }
        let path = format!(
            ".astral/{}/{name}/projection.toml",
            snapshot.manifest.projections
        );
        if let Some(bytes) = snapshot.files.get(&path) {
            let projection: ProjectionManifest = toml::from_str(
                std::str::from_utf8(bytes)
                    .map_err(|_| fail("INVALID_UTF8", "projection manifest must be UTF-8"))?,
            )
            .map_err(|_| fail("INVALID_MANIFEST", "projection manifest is invalid"))?;
            if projection.id != name
                || !matches!(
                    projection.kind.as_str(),
                    "native-checkpoint"
                        | "fresh-context"
                        | "reviewable"
                        | "reviewable-design-context"
                )
                || projection
                    .subsystems
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    != scope
            {
                return Err(fail(
                    "PUBLICATION_CONFLICT",
                    "existing projection identity, type or subsystem scope conflicts with publication",
                ));
            }
        }
        let (staging, _) = staging(&repository)?;
        let projections = directory(&repository.astral, &snapshot.manifest.projections, false)?;
        same_device(&staging, &projections)?;
        if exists(&projections, name)? {
            let projection = open(&projections, name, true)?;
            same_device(&staging, &projection)?;
            if exists(&projection, "bundles")? {
                same_device(&staging, &open(&projection, "bundles", true)?)?;
            }
        }
        snapshot.unchanged(&repository)
    }

    pub(super) fn publish(
        root: &Path,
        name: &str,
        subsystem_ids: &[String],
        bundle: &NativeBundle,
        handoff_text: Option<&str>,
    ) -> Result<PublicationReceipt> {
        identifier(name)?;
        if subsystem_ids.len() > Limits::default().entries {
            return Err(fail("LIMIT_EXCEEDED", "projection subsystem limit"));
        }
        let mut scope = BTreeSet::new();
        for id in subsystem_ids {
            identifier(id)?;
            if !scope.insert(id.clone()) {
                return Err(fail(
                    "DUPLICATE_REFERENCE",
                    "projection subsystem scope contains duplicates",
                ));
            }
        }
        let bundle = NativeBundle::validate(bundle.manifest_bytes(), bundle.payload_bytes())?;
        crate::native_import::validate(&bundle)?;
        let repository = Repository::lock(root)?;
        let snapshot = Snapshot::load(&repository)?;
        if bundle.manifest().source.project_id != snapshot.manifest.id {
            return Err(fail(
                "NATIVE_PROJECT_MISMATCH",
                "saved native bundle belongs to another project",
            ));
        }
        if scope
            .iter()
            .any(|id| !snapshot.manifest.subsystems.contains_key(id))
        {
            return Err(fail(
                "MISSING_REFERENCE",
                "saved projection names an unknown subsystem",
            ));
        }
        let directory_path = format!(".astral/{}/{name}", snapshot.manifest.projections);
        let projection_path = format!("{directory_path}/projection.toml");
        let projection_root = directory(&repository.astral, &snapshot.manifest.projections, false)?;
        let existing = exists(&projection_root, name)?;
        let digest = crate::hash(bundle.manifest_bytes());
        let relative_bundle = format!("bundles/{digest}");
        let bundle_directory = format!("{directory_path}/{relative_bundle}");
        let bundle_path = format!("{bundle_directory}/manifest.json");
        let payload_path = format!("{bundle_directory}/window.json");
        let mut files = snapshot.files.clone();
        let mut projection = if existing {
            let bytes = snapshot.files.get(&projection_path).ok_or_else(|| {
                fail(
                    "PUBLICATION_CONFLICT",
                    "existing projection is not a declared valid context",
                )
            })?;
            let projection: ProjectionManifest = toml::from_str(
                std::str::from_utf8(bytes)
                    .map_err(|_| fail("INVALID_UTF8", "projection manifest must be UTF-8"))?,
            )
            .map_err(|_| fail("INVALID_MANIFEST", "projection manifest is invalid"))?;
            if projection.id != name
                || !matches!(
                    projection.kind.as_str(),
                    "native-checkpoint"
                        | "fresh-context"
                        | "reviewable"
                        | "reviewable-design-context"
                )
                || projection
                    .subsystems
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>()
                    != scope
            {
                return Err(fail(
                    "PUBLICATION_CONFLICT",
                    "existing projection identity, type or subsystem scope conflicts with publication",
                ));
            }
            if handoff_text.is_some() {
                return Err(fail(
                    "PUBLICATION_CONFLICT",
                    "existing projection handoff is preserved; do not supply replacement text",
                ));
            }
            projection
        } else {
            let handoff = handoff_text.unwrap_or("# Saved native project context\n\nThis projection references a locally validated native checkpoint bundle. Runtime tools, permissions, workspace and account routing are supplied at launch. Recorded commands and test outcomes are historical until verified in the destination workspace.\n");
            if handoff.len() > MAX_HANDOFF || handoff.trim().is_empty() {
                return Err(fail(
                    "LIMIT_EXCEEDED",
                    "new projection handoff must be nonempty and at most 32 KiB",
                ));
            }
            files.insert(
                format!("{directory_path}/handoff.md"),
                handoff.as_bytes().to_vec(),
            );
            ProjectionManifest {
                schema_version: 1,
                schema_status: None,
                id: name.into(),
                kind: "native-checkpoint".into(),
                subsystems: subsystem_ids.to_vec(),
                handoff: "handoff.md".into(),
                native_payload_in_repository: true,
                native_bundle: None,
                sources: Vec::new(),
            }
        };
        projection.kind = "native-checkpoint".into();
        projection.native_payload_in_repository = true;
        projection.native_bundle = Some(NativeBundleReference {
            manifest: format!("{relative_bundle}/manifest.json"),
            sha256: digest,
        });
        let projection_bytes = toml::to_string_pretty(&projection)
            .map_err(|_| fail("INVALID_MANIFEST", "cannot encode projection manifest"))?
            .into_bytes();
        files.insert(projection_path.clone(), projection_bytes.clone());
        files.insert(bundle_path.clone(), bundle.manifest_bytes().to_vec());
        files.insert(payload_path.clone(), bundle.payload_bytes().to_vec());
        // Replaced native files stay immutable in the real tree but do not count
        // as selected inputs in the prospective index's aggregate budget.
        if existing {
            let old: ProjectionManifest = toml::from_str(
                std::str::from_utf8(&snapshot.files[&projection_path])
                    .map_err(|_| fail("INVALID_UTF8", "projection manifest must be UTF-8"))?,
            )
            .map_err(|_| fail("INVALID_MANIFEST", "projection manifest is invalid"))?;
            if let Some(reference) = old.native_bundle {
                let old_manifest = format!("{directory_path}/{}", reference.manifest);
                let old_payload = format!(
                    "{}/window.json",
                    old_manifest
                        .rsplit_once('/')
                        .expect("validated native reference has directory")
                        .0
                );
                if old_manifest != bundle_path {
                    files.remove(&old_manifest);
                }
                if old_payload != payload_path {
                    files.remove(&old_payload);
                }
            }
        }
        let (preview_root, preview_path) = preview(&repository, &files)?;
        same_device(&preview_root, &projection_root)?;
        Project::load(&preview_path)?.launch_context(&format!("projection:{name}"), None)?;
        snapshot.unchanged(&repository)?;
        if existing {
            let live_projection = directory(&repository.root, &directory_path, false)?;
            same_device(&preview_root, &live_projection)?;
            let bundles = directory(&live_projection, "bundles", true)?;
            same_device(&preview_root, &bundles)?;
            let digest = crate::hash(bundle.manifest_bytes());
            if exists(&bundles, &digest)? {
                if read(
                    &repository.root,
                    &bundle_path,
                    crate::native_bundle::MAX_MANIFEST_BYTES,
                )? != bundle.manifest_bytes()
                    || read(
                        &repository.root,
                        &payload_path,
                        crate::native_bundle::MAX_PAYLOAD_BYTES,
                    )? != bundle.payload_bytes()
                {
                    return Err(fail(
                        "IMMUTABLE_BUNDLE_CONFLICT",
                        "immutable bundle destination contains different bytes",
                    ));
                }
            } else {
                rename(
                    &preview_root,
                    &bundle_directory,
                    &repository.root,
                    &bundle_directory,
                    true,
                )?;
            }
            snapshot.unchanged(&repository)?;
            rename(
                &preview_root,
                &projection_path,
                &repository.root,
                &projection_path,
                false,
            )?;
        } else {
            rename(
                &preview_root,
                &directory_path,
                &repository.root,
                &directory_path,
                true,
            )?;
        }
        Ok(PublicationReceipt {
            projection: name.into(),
            projection_manifest: handle(projection_path, &projection_bytes),
            bundle_manifest: handle(bundle_path, bundle.manifest_bytes()),
            payload: handle(payload_path, bundle.payload_bytes()),
        })
    }

    pub(super) fn carry_work_file(source: &Path, target: &Path) -> Result<()> {
        let source = Repository::lock(source)?;
        let target = Repository::lock(target)?;
        let before_source = Snapshot::load(&source)?;
        let before_target = Snapshot::load(&target)?;
        if before_source.manifest.id != before_target.manifest.id
            || before_source.manifest.work_items != before_target.manifest.work_items
        {
            return Err(fail(
                "WORK_CARRY_CONFLICT",
                "source and target must identify the same project and work-file path",
            ));
        }
        let path = format!(".astral/{}", before_source.manifest.work_items);
        let bytes = &before_source.files[&path];
        if *bytes == before_target.files[&path] {
            before_source.unchanged(&source)?;
            return before_target.unchanged(&target);
        }
        let mut files = before_target.files.clone();
        files.insert(path.clone(), bytes.clone());
        let (preview, _) = preview(&target, &files)?;
        let (work_parent, _) = parent(&target.root, &path, false)?;
        same_device(&preview, &work_parent)?;
        before_source.unchanged(&source)?;
        before_target.unchanged(&target)?;
        rename(&preview, &path, &target.root, &path, false)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn source_comparison_rejects_changed_work_bytes_before_atomic_replace() {
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir(root.path().join(".astral")).unwrap();
            std::fs::write(root.path().join(".astral/work.jsonl"), b"initial bytes").unwrap();
            let repository = Repository::lock(root.path()).unwrap();
            let snapshot = Snapshot {
                manifest: ProjectManifest {
                    schema_version: 1,
                    schema_status: None,
                    id: "fixture".into(),
                    name: "Fixture".into(),
                    description: "Synthetic comparison".into(),
                    core: "core".into(),
                    projections: "projections".into(),
                    work_items: "work.jsonl".into(),
                    identity: crate::project::Identity {
                        scope: "local".into(),
                        runtime_bindings: "external".into(),
                    },
                    subsystems: BTreeMap::new(),
                },
                files: BTreeMap::from([(".astral/work.jsonl".into(), b"initial bytes".to_vec())]),
                contexts: serde_json::json!([]),
            };
            // This deliberately ignores the advisory lock, as an editor can.
            std::fs::write(root.path().join(".astral/work.jsonl"), b"concurrent edit").unwrap();
            assert_eq!(
                snapshot.unchanged(&repository).unwrap_err().code,
                "PUBLICATION_STALE"
            );
            assert_eq!(
                std::fs::read(root.path().join(".astral/work.jsonl")).unwrap(),
                b"concurrent edit"
            );
        }
    }
}
