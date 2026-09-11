//! Bounded work records, optimistic updates, and record-aware three-way merges.
//!
//! Digests cover exact UTF-8 record lines (excluding CR/LF); semantic merge
//! equality uses the deserialized model, ignoring JSON formatting/key order and
//! normalizing absent optional fields. Array order is significant. Merges emit
//! canonical JSONL ordered by ID; mutations retain unchanged record line bytes
//! so unrelated edits do not invalidate record digests. IDs are never renumbered.
//!
//! Persistence currently requires Unix. Cooperating writers lock the `.astral`
//! directory inode until replacement finishes; no lock file or Git configuration
//! is installed. Descriptor-relative access rejects links and special files.
//! Comparison before rename detects observed external edits, but cannot exclude
//! an uncooperative writer in the final comparison/rename window. Directory sync
//! is best effort: success promises atomic visibility, not crash durability.

#[cfg(unix)]
use crate::project::{Limits, Project, ProjectManifest};
use crate::project::{WorkItem, WorkStatus};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const MAX_FILE_BYTES: usize = 1_048_576;
pub const MAX_RECORDS: usize = 4_096;
/// Records plus dependency, acceptance, and evidence entries, matching Project.
pub const MAX_ENTRIES: usize = 4_096;
pub const MAX_GRAPH_DEPTH: usize = 64;

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: &'static str,
    /// Logical IDs only; errors never include source rows or parser excerpts.
    pub ids: Vec<String>,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;

fn fail(code: &'static str, message: &'static str) -> Error {
    Error {
        code,
        message,
        ids: Vec::new(),
    }
}

fn at_id(code: &'static str, message: &'static str, id: &str) -> Error {
    let mut error = fail(code, message);
    error.ids.push(id.to_owned());
    error
}

#[derive(Debug, Clone, Serialize)]
pub struct Record {
    pub item: WorkItem,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkFile {
    /// Repository-relative path selected by `.astral/project.toml`.
    pub path: String,
    pub digest: String,
    /// Sorted by logical ID, independently of input line order.
    pub records: Vec<Record>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Mutation {
    pub record: Record,
    pub file: WorkFile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    ConcurrentAddition,
    DivergentEdit,
    ModifyDelete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    pub id: String,
    pub kind: ConflictKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum MergeResult {
    Merged {
        bytes: Vec<u8>,
        records: Vec<Record>,
    },
    /// No partial merged file is returned when any record conflicts.
    Conflicts { conflicts: Vec<Conflict> },
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

fn relative(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 4_096
        || value.contains(['\\', ':', '\0'])
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(fail("UNSAFE_PATH", "expected a confined relative path"));
    }
    Ok(())
}

fn distinct(values: &[String]) -> bool {
    let mut seen = BTreeSet::new();
    values
        .iter()
        .all(|value| !value.trim().is_empty() && seen.insert(value))
}

fn entry_count(records: &[Record]) -> usize {
    records
        .iter()
        .map(|record| {
            1 + record.item.depends_on.len()
                + record.item.acceptance.len()
                + record.item.evidence.as_ref().map_or(0, Vec::len)
        })
        .sum()
}

fn validate(records: &[Record]) -> Result<()> {
    if records.len() > MAX_RECORDS || entry_count(records) > MAX_ENTRIES {
        return Err(fail("LIMIT_EXCEEDED", "work record or entry count limit"));
    }
    let mut graph = BTreeMap::new();
    for record in records {
        let item = &record.item;
        if item.schema_version != 1 {
            return Err(fail("UNSUPPORTED_SCHEMA", "work schema_version must be 1"));
        }
        if !identifier(&item.id) || !item.depends_on.iter().all(|id| identifier(id)) {
            return Err(fail("INVALID_ID", "invalid logical work identifier"));
        }
        if item.title.trim().is_empty()
            || item.acceptance.is_empty()
            || !distinct(&item.acceptance)
            || !distinct(&item.depends_on)
        {
            return Err(at_id(
                "INVALID_FIELD",
                "title and distinct acceptance/dependency fields must be nonempty",
                &item.id,
            ));
        }
        if let Some(evidence) = &item.evidence {
            for path in evidence {
                relative(path)?;
            }
        }
        if graph
            .insert(item.id.as_str(), item.depends_on.as_slice())
            .is_some()
        {
            return Err(at_id("DUPLICATE_ID", "duplicate work identifier", &item.id));
        }
    }
    for (id, dependencies) in &graph {
        for dependency in *dependencies {
            if !graph.contains_key(dependency.as_str()) {
                let mut error = at_id("MISSING_REFERENCE", "unknown work dependency", id);
                error.ids.push(dependency.clone());
                return Err(error);
            }
        }
    }
    fn visit<'a>(
        id: &'a str,
        graph: &BTreeMap<&'a str, &'a [String]>,
        visiting: &mut BTreeSet<&'a str>,
        depths: &mut BTreeMap<&'a str, usize>,
    ) -> Result<usize> {
        if visiting.contains(id) {
            return Err(at_id("DEPENDENCY_CYCLE", "cyclic work dependency", id));
        }
        if visiting.len() >= MAX_GRAPH_DEPTH {
            return Err(at_id("LIMIT_EXCEEDED", "work dependency depth limit", id));
        }
        if let Some(depth) = depths.get(id) {
            if visiting.len() + depth > MAX_GRAPH_DEPTH {
                return Err(at_id("LIMIT_EXCEEDED", "work dependency depth limit", id));
            }
            return Ok(*depth);
        }
        visiting.insert(id);
        let mut depth = 1;
        for dependency in graph[id] {
            depth = depth.max(1 + visit(dependency, graph, visiting, depths)?);
        }
        visiting.remove(id);
        depths.insert(id, depth);
        Ok(depth)
    }
    let mut depths = BTreeMap::new();
    for id in graph.keys() {
        visit(id, &graph, &mut BTreeSet::new(), &mut depths)?;
    }
    Ok(())
}

/// Strict schema/UTF-8 JSONL validation, including references and graph bounds.
/// Empty files are valid; blank/comment lines and duplicate JSON fields are not.
pub fn parse(bytes: &[u8]) -> Result<Vec<Record>> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(fail("LIMIT_EXCEEDED", "work file byte limit"));
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| fail("INVALID_UTF8", "work file must be UTF-8"))?;
    let mut records = Vec::new();
    for line in text.lines() {
        if records.len() >= MAX_RECORDS {
            return Err(fail("LIMIT_EXCEEDED", "work record count limit"));
        }
        let item: WorkItem = serde_json::from_str(line)
            .map_err(|_| fail("INVALID_WORK_ITEM", "invalid JSON or work record schema"))?;
        records.push(Record {
            item,
            digest: crate::hash(line.as_bytes()),
        });
    }
    validate(&records)?;
    records.sort_by(|a, b| a.item.id.cmp(&b.item.id));
    Ok(records)
}

fn encoded(records: &[Record], original: Option<&[u8]>) -> Result<Vec<u8>> {
    validate(records)?;
    let mut original_lines = BTreeMap::new();
    if let Some(original) = original {
        // The source was already parsed and validated by State::open.
        for line in std::str::from_utf8(original)
            .expect("validated UTF-8")
            .lines()
        {
            let item: WorkItem = serde_json::from_str(line).expect("validated work record");
            let semantic = serde_json::to_vec(&item).expect("WorkItem serializes");
            original_lines.insert(item.id, (semantic, line));
        }
    }
    let mut sorted: Vec<_> = records.iter().collect();
    sorted.sort_by(|a, b| a.item.id.cmp(&b.item.id));
    let mut bytes = Vec::new();
    // Bound serialization even for caller-supplied creation strings.
    struct Bounded<'a>(&'a mut Vec<u8>);
    impl std::io::Write for Bounded<'_> {
        fn write(&mut self, value: &[u8]) -> std::io::Result<usize> {
            if value.len() > MAX_FILE_BYTES.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("work file byte limit"));
            }
            self.0.extend_from_slice(value);
            Ok(value.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    for record in sorted {
        if let Some((_, line)) = original_lines
            .get(&record.item.id)
            .filter(|(old, _)| *old == semantic(Some(record)).expect("present record"))
        {
            std::io::Write::write_all(&mut Bounded(&mut bytes), line.as_bytes())
                .map_err(|_| fail("LIMIT_EXCEEDED", "serialized work file byte limit"))?;
        } else {
            serde_json::to_writer(Bounded(&mut bytes), &record.item)
                .map_err(|_| fail("LIMIT_EXCEEDED", "serialized work file byte limit"))?;
        }
        if bytes.len() == MAX_FILE_BYTES {
            return Err(fail("LIMIT_EXCEEDED", "serialized work file byte limit"));
        }
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn semantic(record: Option<&Record>) -> Option<Vec<u8>> {
    record.map(|record| serde_json::to_vec(&record.item).expect("WorkItem serializes"))
}

/// Pure record-level three-way merge. Each input and the successful output must
/// validate independently. The three inputs together are bounded to 3 MiB.
/// Deletions here are merge semantics; the update API cannot delete records.
/// This byte-only API cannot check references from subsystem manifests.
pub fn merge(base: &[u8], ours: &[u8], theirs: &[u8]) -> Result<MergeResult> {
    let base = parse(base)?;
    let ours = parse(ours)?;
    let theirs = parse(theirs)?;
    fn indexed(records: &[Record]) -> BTreeMap<&str, &Record> {
        records.iter().map(|r| (r.item.id.as_str(), r)).collect()
    }
    let base = indexed(&base);
    let ours = indexed(&ours);
    let theirs = indexed(&theirs);
    let ids: BTreeSet<_> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .collect();
    let mut merged = Vec::new();
    let mut conflicts = Vec::new();
    for id in ids {
        let b = base.get(id).copied();
        let o = ours.get(id).copied();
        let t = theirs.get(id).copied();
        let (bv, ov, tv) = (semantic(b), semantic(o), semantic(t));
        let selected = if ov == tv {
            o
        } else if ov == bv {
            t
        } else if tv == bv {
            o
        } else {
            conflicts.push(Conflict {
                id: (*id).to_owned(),
                kind: if b.is_none() {
                    ConflictKind::ConcurrentAddition
                } else if o.is_none() || t.is_none() {
                    ConflictKind::ModifyDelete
                } else {
                    ConflictKind::DivergentEdit
                },
            });
            continue;
        };
        if let Some(record) = selected {
            merged.push(record.clone());
        }
    }
    if !conflicts.is_empty() {
        return Ok(MergeResult::Conflicts { conflicts });
    }
    let bytes = encoded(&merged, None)?;
    let records = parse(&bytes)?;
    Ok(MergeResult::Merged { bytes, records })
}

/// Read and validate the existing project and its selected work register.
pub fn read(root: impl AsRef<Path>) -> Result<WorkFile> {
    #[cfg(unix)]
    {
        let state = unix::State::open(root.as_ref(), false)?;
        state.validate_project(&state.work.bytes, &state.file.records)?;
        state.unchanged()?;
        Ok(state.file.clone())
    }
    #[cfg(not(unix))]
    {
        let _ = root;
        Err(fail(
            "UNSUPPORTED_PLATFORM",
            "work persistence requires Unix",
        ))
    }
}

/// Create one open record with a fresh OS-random AST ID; existing IDs are retained.
pub fn create(
    root: impl AsRef<Path>,
    title: &str,
    acceptance: &[String],
    depends_on: &[String],
) -> Result<Mutation> {
    if title.len() > MAX_FILE_BYTES
        || acceptance.len().saturating_add(depends_on.len()) >= MAX_ENTRIES
        || acceptance
            .iter()
            .chain(depends_on)
            .try_fold(title.len(), |sum, value| sum.checked_add(value.len()))
            .is_none_or(|size| size > MAX_FILE_BYTES)
    {
        return Err(fail("LIMIT_EXCEEDED", "new work record input limit"));
    }
    mutate(root.as_ref(), |records| {
        let id = crate::work::generate_id(records.iter().map(|r| r.item.id.as_str()))
            .map_err(|error| fail(error.code(), "work ID allocation failed"))?;
        records.push(Record {
            item: WorkItem {
                schema_version: 1,
                id: id.clone(),
                title: title.to_owned(),
                status: WorkStatus::Open,
                depends_on: depends_on.to_vec(),
                acceptance: acceptance.to_vec(),
                evidence: None,
                result: None,
                provenance: None,
                scope: None,
                remaining: None,
            },
            digest: String::new(),
        });
        Ok(id)
    })
}

/// Change status only, retaining every other model field. The digest must match
/// the exact observed record line; unrelated record edits do not make it stale.
pub fn update(
    root: impl AsRef<Path>,
    id: &str,
    status: WorkStatus,
    expected_digest: &str,
) -> Result<Mutation> {
    if !identifier(id) {
        return Err(fail("INVALID_ID", "invalid logical work identifier"));
    }
    if expected_digest.len() != 64
        || !expected_digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(fail("INVALID_DIGEST", "expected a SHA-256 record digest"));
    }
    mutate(root.as_ref(), |records| {
        let record = records
            .iter_mut()
            .find(|record| record.item.id == id)
            .ok_or_else(|| at_id("MISSING_WORK_ITEM", "unknown work identifier", id))?;
        if record.digest != expected_digest {
            return Err(at_id("STALE_WORK_ITEM", "work record digest changed", id));
        }
        record.item.status = status;
        Ok(id.to_owned())
    })
}

fn mutate(
    root: &Path,
    change: impl FnOnce(&mut Vec<Record>) -> Result<String>,
) -> Result<Mutation> {
    #[cfg(unix)]
    {
        let state = unix::State::open(root, true)?;
        let mut records = state.file.records.clone();
        let id = change(&mut records)?;
        let bytes = encoded(&records, Some(&state.work.bytes))?;
        let records = parse(&bytes)?;
        // Validate the candidate in memory and reserve its growth while checking
        // the existing project. No temporary replacement of the live work file.
        state.validate_project(&bytes, &records)?;
        let file = WorkFile {
            path: state.file.path.clone(),
            digest: crate::hash(&bytes),
            records,
        };
        let record = file
            .records
            .iter()
            .find(|record| record.item.id == id)
            .expect("mutation retains its target ID")
            .clone();
        state.replace(&bytes)?;
        Ok(Mutation { record, file })
    }
    #[cfg(not(unix))]
    {
        let _ = (root, change);
        Err(fail(
            "UNSUPPORTED_PLATFORM",
            "work persistence requires Unix",
        ))
    }
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::ffi::CString;
    use std::fs::{File, Metadata, OpenOptions, Permissions};
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::path::PathBuf;

    fn io_error() -> Error {
        fail("WORK_IO", "work file operation failed")
    }

    fn changed() -> Error {
        fail(
            "FILESYSTEM_CHANGED",
            "work inputs or directory identity changed",
        )
    }

    fn name(value: &str) -> Result<CString> {
        CString::new(value).map_err(|_| fail("UNSAFE_PATH", "invalid path component"))
    }

    fn identity(metadata: &Metadata) -> (u64, u64) {
        (metadata.dev(), metadata.ino())
    }

    fn regular(metadata: &Metadata) -> Result<()> {
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(fail(
                "UNSAFE_FILE",
                "expected a regular file with exactly one hard link",
            ));
        }
        Ok(())
    }

    fn root_file(root: &Path) -> Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)
            .map_err(|_| fail("INVALID_ROOT", "repository root is unavailable"))
    }

    fn open_at(parent: &File, component: &str, directory: bool) -> Result<File> {
        let name = name(component)?;
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: live directory descriptor, terminated name, writable stat.
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(fail("PATH_UNAVAILABLE", "work path is unavailable"));
        }
        // SAFETY: successful fstatat initialized stat.
        let stat = unsafe { stat.assume_init() };
        let expected_kind = if directory {
            libc::S_IFDIR
        } else {
            libc::S_IFREG
        };
        if stat.st_mode & libc::S_IFMT != expected_kind || (!directory && stat.st_nlink != 1) {
            return Err(fail(
                "UNSAFE_FILE",
                "links and nonregular work paths are rejected",
            ));
        }
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory { libc::O_DIRECTORY } else { 0 };
        // SAFETY: live parent descriptor and terminated name.
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(fail(
                "PATH_UNAVAILABLE",
                "work path could not be safely opened",
            ));
        }
        // SAFETY: this File exclusively owns the new descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata().map_err(|_| io_error())?;
        #[allow(clippy::unnecessary_cast)]
        let expected_identity = (stat.st_dev as u64, stat.st_ino as u64);
        if identity(&metadata) != expected_identity {
            return Err(changed());
        }
        if directory {
            if !metadata.is_dir() {
                return Err(changed());
            }
        } else {
            regular(&metadata)?;
        }
        Ok(file)
    }

    fn parent_of(astral: &File, path: &str) -> Result<(File, String)> {
        relative(path)?;
        let mut components = path.split('/').peekable();
        let mut parent = astral.try_clone().map_err(|_| io_error())?;
        while let Some(component) = components.next() {
            if components.peek().is_none() {
                return Ok((parent, component.to_owned()));
            }
            parent = open_at(&parent, component, true)?;
        }
        unreachable!("relative rejects empty paths")
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Stamp {
        identity: (u64, u64),
        len: u64,
        modified: (i64, i64),
        changed: (i64, i64),
        mode: u32,
        owner: (u32, u32),
    }

    impl Stamp {
        fn read(file: &File) -> Result<Self> {
            let metadata = file.metadata().map_err(|_| io_error())?;
            regular(&metadata)?;
            Ok(Self {
                identity: identity(&metadata),
                len: metadata.len(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
                mode: metadata.mode(),
                owner: (metadata.uid(), metadata.gid()),
            })
        }
    }

    pub(super) struct Snapshot {
        stamp: Stamp,
        pub bytes: Vec<u8>,
    }

    impl Snapshot {
        fn read(parent: &File, component: &str) -> Result<Self> {
            let mut file = open_at(parent, component, false)?;
            let stamp = Stamp::read(&file)?;
            if stamp.len > MAX_FILE_BYTES as u64 {
                return Err(fail("LIMIT_EXCEEDED", "work input file byte limit"));
            }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(MAX_FILE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| io_error())?;
            if bytes.len() > MAX_FILE_BYTES {
                return Err(fail("LIMIT_EXCEEDED", "work input file byte limit"));
            }
            if Stamp::read(&file)? != stamp || bytes.len() as u64 != stamp.len {
                return Err(changed());
            }
            Ok(Self { stamp, bytes })
        }

        fn matches(&self, other: &Self) -> Result<()> {
            if self.stamp != other.stamp || self.bytes != other.bytes {
                return Err(changed());
            }
            Ok(())
        }
    }

    pub(super) struct State {
        root_path: PathBuf,
        root: File,
        // Holds the advisory directory lock for the entire mutation.
        astral: File,
        locked: bool,
        parent: File,
        work_path: String,
        filename: String,
        manifest: Snapshot,
        pub work: Snapshot,
        pub file: WorkFile,
    }

    impl State {
        pub fn open(root: &Path, lock: bool) -> Result<Self> {
            // The caller's root aliases are resolved once, as in Project::load.
            let root_path = root
                .canonicalize()
                .map_err(|_| fail("INVALID_ROOT", "repository root is unavailable"))?;
            let root = root_file(&root_path)?;
            let astral = open_at(&root, ".astral", true)?;
            if lock {
                fs2::FileExt::try_lock_exclusive(&astral).map_err(|error| {
                    if error.kind() == std::io::ErrorKind::WouldBlock {
                        fail(
                            "WORK_LOCKED",
                            "another cooperating writer holds the work lock",
                        )
                    } else {
                        fail("WORK_LOCK_FAILED", "work directory advisory lock failed")
                    }
                })?;
            }
            let manifest = Snapshot::read(&astral, "project.toml")?;
            let text = std::str::from_utf8(&manifest.bytes)
                .map_err(|_| fail("INVALID_UTF8", "project manifest must be UTF-8"))?;
            let parsed: ProjectManifest = toml::from_str(text)
                .map_err(|_| fail("INVALID_MANIFEST", "invalid project manifest schema"))?;
            if parsed.schema_version != 1 {
                return Err(fail(
                    "UNSUPPORTED_SCHEMA",
                    "project schema_version must be 1",
                ));
            }
            let work_path = parsed.work_items;
            let (parent, filename) = parent_of(&astral, &work_path)?;
            let work = Snapshot::read(&parent, &filename)?;
            let records = parse(&work.bytes)?;
            let file = WorkFile {
                path: format!(".astral/{work_path}"),
                digest: crate::hash(&work.bytes),
                records,
            };
            Ok(Self {
                root_path,
                root,
                astral,
                locked: lock,
                parent,
                work_path,
                filename,
                manifest,
                work,
                file,
            })
        }

        pub fn validate_project(&self, bytes: &[u8], records: &[Record]) -> Result<()> {
            let mut limits = Limits::default();
            // Existing subsystem references remain valid: create adds an ID;
            // update changes only status. Reserve candidate growth against the
            // resolver's aggregate limits while it reads the ORIGINAL file.
            limits.total_bytes -= bytes.len().saturating_sub(self.work.bytes.len());
            limits.entries -= entry_count(records).saturating_sub(entry_count(&self.file.records));
            Project::load_with_limits(&self.root_path, limits).map_err(|error| {
                fail(
                    error.code,
                    "existing project or candidate aggregate is invalid",
                )
            })?;
            Ok(())
        }

        pub fn unchanged(&self) -> Result<()> {
            let current_root = root_file(&self.root_path)?;
            if identity(&current_root.metadata().map_err(|_| io_error())?)
                != identity(&self.root.metadata().map_err(|_| io_error())?)
            {
                return Err(changed());
            }
            let astral = open_at(&current_root, ".astral", true)?;
            if identity(&astral.metadata().map_err(|_| io_error())?)
                != identity(&self.astral.metadata().map_err(|_| io_error())?)
            {
                return Err(changed());
            }
            self.manifest
                .matches(&Snapshot::read(&astral, "project.toml")?)?;
            let (parent, filename) = parent_of(&astral, &self.work_path)?;
            if identity(&parent.metadata().map_err(|_| io_error())?)
                != identity(&self.parent.metadata().map_err(|_| io_error())?)
            {
                return Err(changed());
            }
            self.work.matches(&Snapshot::read(&parent, &filename)?)
        }

        pub fn replace(&self, bytes: &[u8]) -> Result<()> {
            let mut temporary = Temporary::create(&self.parent)?;
            temporary.file.write_all(bytes).map_err(|_| io_error())?;
            temporary
                .file
                .set_permissions(Permissions::from_mode(self.work.stamp.mode & 0o777))
                .map_err(|_| io_error())?;
            temporary.file.sync_all().map_err(|_| io_error())?;
            let expected = Stamp::read(&temporary.file)?;
            self.unchanged()?;
            let actual = Snapshot::read(&self.parent, &temporary.filename)?;
            if actual.stamp != expected || actual.bytes != bytes {
                return Err(changed());
            }
            let source = name(&temporary.filename)?;
            let target = name(&self.filename)?;
            // SAFETY: both names are terminated, and the held parent is a live
            // directory descriptor. Rename is atomic within this directory.
            if unsafe {
                libc::renameat(
                    self.parent.as_raw_fd(),
                    source.as_ptr(),
                    self.parent.as_raw_fd(),
                    target.as_ptr(),
                )
            } != 0
            {
                return Err(io_error());
            }
            temporary.renamed = true;
            // No ordinary error is returned after replacing the original.
            let _ = self.parent.sync_all();
            Ok(())
        }
    }

    impl Drop for State {
        fn drop(&mut self) {
            if self.locked {
                let _ = fs2::FileExt::unlock(&self.astral);
            }
        }
    }

    struct Temporary<'a> {
        parent: &'a File,
        filename: String,
        file: File,
        renamed: bool,
    }

    impl<'a> Temporary<'a> {
        fn create(parent: &'a File) -> Result<Self> {
            for _ in 0..16 {
                let id = crate::work::generate_id(std::iter::empty())
                    .map_err(|error| fail(error.code(), "temporary name allocation failed"))?;
                let filename = format!(".astral-work-{id}.tmp");
                let name = name(&filename)?;
                // SAFETY: live parent descriptor, terminated name, explicit
                // create mode. O_EXCL never overwrites an existing entry.
                let fd = unsafe {
                    libc::openat(
                        parent.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_WRONLY
                            | libc::O_CREAT
                            | libc::O_EXCL
                            | libc::O_NOFOLLOW
                            | libc::O_CLOEXEC,
                        0o600,
                    )
                };
                if fd >= 0 {
                    return Ok(Self {
                        parent,
                        filename,
                        // SAFETY: this File exclusively owns the new descriptor.
                        file: unsafe { File::from_raw_fd(fd) },
                        renamed: false,
                    });
                }
                if std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists {
                    return Err(io_error());
                }
            }
            Err(fail(
                "WORK_TEMP_COLLISION",
                "temporary name attempts exhausted",
            ))
        }
    }

    impl Drop for Temporary<'_> {
        fn drop(&mut self) {
            if !self.renamed {
                // Delete only the temporary inode this operation created. If an
                // external actor swapped its name, leave that entry untouched.
                if let Ok(file) = open_at(self.parent, &self.filename, false) {
                    if let (Ok(actual), Ok(owned), Ok(name)) =
                        (file.metadata(), self.file.metadata(), name(&self.filename))
                    {
                        if identity(&actual) == identity(&owned) {
                            // SAFETY: live parent descriptor and terminated name.
                            unsafe { libc::unlinkat(self.parent.as_raw_fd(), name.as_ptr(), 0) };
                        }
                    }
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn fixture() -> tempfile::TempDir {
            let fixture = tempfile::tempdir().unwrap();
            std::fs::create_dir(fixture.path().join(".astral")).unwrap();
            std::fs::write(
                fixture.path().join(".astral/project.toml"),
                "schema_version=1\nid='fixture'\nname='Fixture'\ndescription='Test'\ncore='core'\nprojections='projections'\nwork_items='items.jsonl'\n[identity]\nscope='local'\nruntime_bindings='private'\n[subsystems]\n",
            ).unwrap();
            let path = fixture.path().join(".astral/items.jsonl");
            std::fs::write(&path, b"").unwrap();
            fixture
        }

        #[test]
        fn compare_before_replace_preserves_external_edit_and_cleans_temporary() {
            let fixture = fixture();
            let path = fixture.path().join(".astral/items.jsonl");
            let state = State::open(fixture.path(), true).unwrap();
            std::fs::write(&path, b"external change\n").unwrap();
            assert_eq!(state.replace(b"").unwrap_err().code, "FILESYSTEM_CHANGED");
            assert_eq!(std::fs::read(&path).unwrap(), b"external change\n");
            assert_eq!(
                std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
                2
            );
        }

        #[test]
        fn replacing_inode_with_identical_bytes_is_detected() {
            let fixture = fixture();
            let path = fixture.path().join(".astral/items.jsonl");
            let state = State::open(fixture.path(), true).unwrap();
            let replacement = fixture.path().join("replacement");
            std::fs::write(&replacement, b"").unwrap();
            std::fs::rename(replacement, &path).unwrap();
            let external_inode = std::fs::metadata(&path).unwrap().ino();
            assert_eq!(state.replace(b"").unwrap_err().code, "FILESYSTEM_CHANGED");
            assert_eq!(std::fs::metadata(&path).unwrap().ino(), external_inode);
            assert_eq!(
                std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
                2
            );
        }

        #[test]
        fn manifest_change_or_rebound_directory_is_detected() {
            for rebind_directory in [false, true] {
                let fixture = fixture();
                let path = fixture.path().join(".astral/items.jsonl");
                let state = State::open(fixture.path(), true).unwrap();
                if rebind_directory {
                    let old = fixture.path().join("old-astral");
                    std::fs::rename(fixture.path().join(".astral"), &old).unwrap();
                    std::fs::create_dir(fixture.path().join(".astral")).unwrap();
                    std::fs::copy(
                        old.join("project.toml"),
                        fixture.path().join(".astral/project.toml"),
                    )
                    .unwrap();
                    std::fs::write(&path, b"").unwrap();
                } else {
                    let manifest = fixture.path().join(".astral/project.toml");
                    let mut bytes = std::fs::read(&manifest).unwrap();
                    bytes.push(b'\n');
                    std::fs::write(manifest, bytes).unwrap();
                }
                assert_eq!(state.replace(b"").unwrap_err().code, "FILESYSTEM_CHANGED");
                assert_eq!(std::fs::read(path).unwrap(), b"");
                let retained = if rebind_directory {
                    "old-astral"
                } else {
                    ".astral"
                };
                assert_eq!(
                    std::fs::read_dir(fixture.path().join(retained))
                        .unwrap()
                        .count(),
                    2
                );
            }
        }
    }
}
