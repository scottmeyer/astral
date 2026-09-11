//! Read-only, bounded inspection of Git-native project context.
//!
//! Handles fingerprint observed source bytes, not an atomic filesystem snapshot
//! or provider-equivalent context. Nothing here launches an agent or binds native
//! checkpoint state. Source provenance is inert data, never executable authority.

use crate::native_bundle::{BundleSummary, NativeBundle};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn error(code: &'static str, message: impl Into<String>) -> Error {
    let message = message.into();
    Error {
        code,
        // Diagnostic output must remain bounded even for a hostile manifest path.
        message: message.chars().take(512).collect(),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub file_bytes: usize,
    pub native_payload_bytes: usize,
    pub total_bytes: usize,
    pub files: usize,
    pub entries: usize,
    pub graph_depth: usize,
    pub output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            file_bytes: 1_048_576,
            native_payload_bytes: crate::native_bundle::MAX_PAYLOAD_BYTES,
            total_bytes: 16_777_216,
            files: 2_048,
            entries: 4_096,
            graph_depth: 64,
            output_bytes: 2_097_152,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub scope: String,
    pub runtime_bindings: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_status: Option<String>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub core: String,
    pub projections: String,
    pub work_items: String,
    pub identity: Identity,
    pub subsystems: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubsystemManifest {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_status: Option<String>,
    pub id: String,
    pub purpose: String,
    pub readme: String,
    pub rules: Vec<String>,
    pub decisions: Vec<String>,
    pub work_items: Vec<String>,
    pub projection: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub id: String,
    pub kind: String,
    pub scope: String,
    pub availability: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locator_policy: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectionManifest {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema_status: Option<String>,
    pub id: String,
    pub kind: String,
    pub subsystems: Vec<String>,
    pub handoff: String,
    pub native_payload_in_repository: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native_bundle: Option<NativeBundleReference>,
    pub sources: Vec<Provenance>,
}

/// Repository-relative immutable artifact reference, never an executable locator.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeBundleReference {
    pub manifest: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Open,
    InProgress,
    Blocked,
    Complete,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkItem {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub status: WorkStatus,
    pub depends_on: Vec<String>,
    pub acceptance: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceHandle {
    pub path: String,
    pub sha256: String,
    pub bytes: usize,
    /// When present, the hash covers this exact JSONL record, without its line ending.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_id: Option<String>,
}

/// Validated bytes are available locally; destination runtime binding is separate.
#[derive(Debug, Clone, Serialize)]
pub struct NativeArtifact {
    pub projection: String,
    pub availability: NativeAvailability,
    pub runtime_binding: NativeRuntimeBinding,
    pub manifest: SourceHandle,
    pub payload: SourceHandle,
    pub bundle: BundleSummary,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeAvailability {
    Validated,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeRuntimeBinding {
    Unbound,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextKind {
    Subsystem,
    Projection,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextSelection {
    pub kind: ContextKind,
    pub id: String,
    pub subsystems: Vec<String>,
    pub projection: Option<String>,
    pub work_item: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FreshDocument {
    pub source: SourceHandle,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FreshWorkItem {
    pub item: WorkItem,
    pub source: SourceHandle,
    /// Exact observed JSONL record, without its line ending.
    pub text: String,
}

/// Selected readable inputs for a fresh session, never a restored native checkpoint.
#[derive(Debug, Clone, Serialize)]
pub struct FreshContext {
    pub schema_version: u32,
    pub project_id: String,
    pub selection: ContextSelection,
    /// Also binds linked projection manifests checked by the fresh-only kind gate,
    /// so this can differ from the metadata-only inspection selection digest.
    pub selection_digest: String,
    /// Includes manifest fingerprints as well as selected document/record handles.
    pub sources: Vec<SourceHandle>,
    /// Core, selected subsystem/dependency documents, and an explicit handoff only.
    pub documents: Vec<FreshDocument>,
    pub work_item: Option<FreshWorkItem>,
}

/// Current selected documents and, when required, a separate immutable native
/// window. Documents never stand in for the required native state.
#[derive(Debug, Clone)]
pub struct ProjectLaunchContext {
    pub current_context: FreshContext,
    pub native: Option<NativeBundle>,
}

struct Subsystem {
    manifest: SubsystemManifest,
    directory: String,
}
struct Projection {
    manifest: ProjectionManifest,
    directory: String,
    native: Option<LoadedNativeBundle>,
}
struct LoadedNativeBundle {
    bundle: NativeBundle,
    manifest_path: String,
    payload_path: String,
}
struct WorkRecord {
    item: WorkItem,
    source: SourceHandle,
    text: String,
}

struct ResolvedSelection<'a> {
    selection: ContextSelection,
    selection_digest: String,
    sources: Vec<SourceHandle>,
    document_paths: BTreeSet<String>,
    projection: Option<&'a Projection>,
    work: Option<&'a WorkRecord>,
}

pub struct Project {
    manifest: ProjectManifest,
    subsystems: BTreeMap<String, Subsystem>,
    projections: BTreeMap<String, Projection>,
    work: BTreeMap<String, WorkRecord>,
    sources: BTreeMap<String, SourceHandle>,
    /// Bounded bytes observed at load time; assembly never reopens these paths.
    contents: BTreeMap<String, Vec<u8>>,
    limits: Limits,
}

fn relative(value: &str) -> Result<()> {
    if value.is_empty()
        || value.contains(['\\', ':', '\0'])
        || value
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
        || Path::new(value).is_absolute()
    {
        return Err(error(
            "UNSAFE_PATH",
            format!("expected a confined relative path: {value:?}"),
        ));
    }
    Ok(())
}

fn join(base: &str, child: &str) -> Result<String> {
    relative(base)?;
    relative(child)?;
    Ok(format!("{base}/{child}"))
}

fn identifier(value: &str) -> Result<()> {
    if value.is_empty()
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(error(
            "INVALID_ID",
            format!("invalid logical identifier: {value:?}"),
        ));
    }
    Ok(())
}

fn nonempty(value: &str, field: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(error("INVALID_FIELD", format!("{field} must not be empty")));
    }
    Ok(())
}

fn version(v: u32, path: &str) -> Result<()> {
    if v != 1 {
        return Err(error(
            "UNSUPPORTED_SCHEMA",
            format!("{path}: schema_version must be 1"),
        ));
    }
    Ok(())
}

fn distinct(values: &[String], field: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        nonempty(value, field)?;
        if !seen.insert(value) {
            return Err(error(
                "DUPLICATE_REFERENCE",
                format!("{field}: duplicate {value:?}"),
            ));
        }
    }
    Ok(())
}

fn schema<T: serde::de::DeserializeOwned>(bytes: &[u8], path: &str) -> Result<T> {
    let text = std::str::from_utf8(bytes).map_err(|_| error("INVALID_UTF8", path))?;
    // Avoid echoing entire source lines or private provenance through parser diagnostics.
    toml::from_str(text).map_err(|_| {
        error(
            "INVALID_MANIFEST",
            format!("{path}: invalid TOML or schema fields"),
        )
    })
}

/// Descriptor-relative filesystem access. No symlink inside the repository is followed.
/// Individual observations are protected; this is not a transactional tree snapshot.
struct Reader {
    root: File,
    limits: Limits,
    total_bytes: usize,
    entries: usize,
    contents: BTreeMap<String, Vec<u8>>,
}

impl Reader {
    fn new(root: &Path, limits: Limits) -> Result<Self> {
        Ok(Self {
            root: confined::root(root)?,
            limits,
            total_bytes: 0,
            entries: 0,
            contents: BTreeMap::new(),
        })
    }

    fn charge(&mut self, count: usize) -> Result<()> {
        self.entries = self
            .entries
            .checked_add(count)
            .ok_or_else(|| error("LIMIT_EXCEEDED", "entry count"))?;
        if self.entries > self.limits.entries {
            return Err(error("LIMIT_EXCEEDED", "entry count"));
        }
        Ok(())
    }

    fn read(&mut self, path: &str) -> Result<Vec<u8>> {
        self.read_limited(path, self.limits.file_bytes)
    }

    fn read_limited(&mut self, path: &str, file_limit: usize) -> Result<Vec<u8>> {
        relative(path)?;
        if let Some(bytes) = self.contents.get(path) {
            if bytes.len() > file_limit {
                return Err(error("LIMIT_EXCEEDED", format!("{path}: file byte limit")));
            }
            return Ok(bytes.clone());
        }
        if self.contents.len() >= self.limits.files {
            return Err(error("LIMIT_EXCEEDED", "file count"));
        }
        let mut file = confined::open(&self.root, path, false)?;
        let remaining = self.limits.total_bytes.saturating_sub(self.total_bytes);
        let limit = file_limit.min(remaining);
        if file
            .metadata()
            .map_err(|_| error("READ_FAILED", path))?
            .len()
            > limit as u64
        {
            return Err(error(
                "LIMIT_EXCEEDED",
                format!("{path}: file or aggregate byte limit"),
            ));
        }
        let mut bytes = Vec::new();
        file.by_ref()
            .take((limit as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| error("READ_FAILED", path))?;
        if bytes.len() > limit {
            return Err(error(
                "LIMIT_EXCEEDED",
                format!("{path}: file or aggregate byte limit"),
            ));
        }
        self.total_bytes += bytes.len();
        self.contents.insert(path.to_owned(), bytes.clone());
        Ok(bytes)
    }

    fn directory(&mut self, path: &str) -> Result<Vec<String>> {
        let file = confined::open(&self.root, path, true)?;
        let names = confined::names(file, self.limits.entries.saturating_sub(self.entries))?;
        self.charge(names.len())?;
        Ok(names)
    }
}

fn load_native_bundle(
    reader: &mut Reader,
    directory: &str,
    projection: &ProjectionManifest,
    project_id: &str,
) -> Result<Option<LoadedNativeBundle>> {
    if projection.kind == "native-checkpoint" && projection.native_bundle.is_none() {
        return Err(error(
            "MISSING_NATIVE_BUNDLE",
            "native-checkpoint projection requires a native_bundle reference",
        ));
    }
    let Some(reference) = &projection.native_bundle else {
        if projection.native_payload_in_repository {
            return Err(error(
                "INVALID_NATIVE_REFERENCE",
                "native_payload_in_repository requires a native-checkpoint bundle reference",
            ));
        }
        return Ok(None);
    };
    if projection.kind != "native-checkpoint" || !projection.native_payload_in_repository {
        return Err(error(
            "INVALID_NATIVE_REFERENCE",
            "a native bundle requires kind=native-checkpoint and native_payload_in_repository=true",
        ));
    }
    if reference.sha256.len() != 64
        || !reference
            .sha256
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            "INVALID_NATIVE_REFERENCE",
            "native bundle reference requires a lowercase SHA-256 digest",
        ));
    }
    let manifest_path = join(directory, &reference.manifest)?;
    let manifest_bytes = reader
        .read_limited(
            &manifest_path,
            reader
                .limits
                .file_bytes
                .min(crate::native_bundle::MAX_MANIFEST_BYTES),
        )
        .map_err(native_availability_error)?;
    if crate::hash(&manifest_bytes) != reference.sha256 {
        return Err(error(
            "NATIVE_BUNDLE_DIGEST_MISMATCH",
            "native manifest differs from its pinned digest",
        ));
    }
    // The version-one payload name is fixed. Neither metadata nor a payload can
    // redirect file reads outside this manifest's confined directory.
    let (parent, _) = manifest_path
        .rsplit_once('/')
        .ok_or_else(|| error("UNSAFE_PATH", "native manifest has no parent directory"))?;
    let payload_path = join(parent, "window.json")?;
    let payload_bytes = reader
        .read_limited(
            &payload_path,
            reader
                .limits
                .native_payload_bytes
                .min(crate::native_bundle::MAX_PAYLOAD_BYTES),
        )
        .map_err(native_availability_error)?;
    let bundle = NativeBundle::validate(&manifest_bytes, &payload_bytes)?;
    if bundle.manifest().source.project_id != project_id {
        return Err(error(
            "NATIVE_PROJECT_MISMATCH",
            "native bundle source project differs from the project index",
        ));
    }
    Ok(Some(LoadedNativeBundle {
        bundle,
        manifest_path,
        payload_path,
    }))
}

fn native_availability_error(failure: Error) -> Error {
    if failure.code == "PATH_UNAVAILABLE" {
        error(
            "NATIVE_BUNDLE_UNAVAILABLE",
            "declared native bundle file is unavailable",
        )
    } else {
        failure
    }
}

fn validate_native_document_roles(
    reader: &Reader,
    core: &str,
    subsystems: &BTreeMap<String, Subsystem>,
    projections: &BTreeMap<String, Projection>,
) -> Result<()> {
    let mut native_hashes = BTreeSet::new();
    for native in projections.values().filter_map(|p| p.native.as_ref()) {
        native_hashes.insert(crate::hash(native.bundle.manifest_bytes()));
        native_hashes.insert(crate::hash(native.bundle.payload_bytes()));
    }
    if native_hashes.is_empty() {
        return Ok(());
    }
    let mut paths = BTreeSet::new();
    for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
        paths.insert(join(core, name)?);
    }
    for subsystem in subsystems.values() {
        for doc in std::iter::once(&subsystem.manifest.readme)
            .chain(subsystem.manifest.rules.iter())
            .chain(subsystem.manifest.decisions.iter())
        {
            paths.insert(join(&subsystem.directory, doc)?);
        }
    }
    for projection in projections.values() {
        paths.insert(join(&projection.directory, &projection.manifest.handoff)?);
    }
    // Comparing observed bytes also covers case aliases, hard links and exact
    // copies. An unrelated native artifact must not become fresh text merely
    // because another subsystem declares it as a README or handoff.
    for path in paths {
        if native_hashes.contains(&crate::hash(&reader.contents[&path])) {
            return Err(error(
                "NATIVE_DOCUMENT_CONFLICT",
                "a readable document contains a declared native artifact; keep document and native roles separate",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
mod confined {
    use super::*;
    use std::ffi::{CStr, CString};
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::os::unix::fs::OpenOptionsExt;

    pub fn root(path: &Path) -> Result<File> {
        // The caller chooses the root. Resolve its aliases once; confinement starts here.
        let path = path
            .canonicalize()
            .map_err(|_| error("INVALID_ROOT", "repository root is unavailable"))?;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
            .map_err(|_| {
                error(
                    "INVALID_ROOT",
                    "repository root must be a readable directory",
                )
            })
    }

    pub fn open(root: &File, path: &str, directory: bool) -> Result<File> {
        relative(path)?;
        let mut parent = root.try_clone().map_err(|_| error("READ_FAILED", path))?;
        let components: Vec<_> = path.split('/').collect();
        for (index, component) in components.iter().enumerate() {
            let is_dir = index + 1 != components.len() || directory;
            let name = CString::new(*component).map_err(|_| error("UNSAFE_PATH", path))?;
            let mut before = std::mem::MaybeUninit::<libc::stat>::uninit();
            // SAFETY: name is terminated, parent is live, and fstatat initializes before on success.
            let rc = unsafe {
                libc::fstatat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    before.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            };
            if rc != 0 {
                return Err(error("PATH_UNAVAILABLE", path));
            }
            // SAFETY: successful fstatat above initialized the structure.
            let before = unsafe { before.assume_init() };
            let kind = before.st_mode & libc::S_IFMT;
            if kind == libc::S_IFLNK {
                return Err(error("SYMLINK_REJECTED", path));
            }
            if kind != if is_dir { libc::S_IFDIR } else { libc::S_IFREG } {
                return Err(error("INVALID_FILE_TYPE", path));
            }
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | libc::O_CLOEXEC
                | if is_dir { libc::O_DIRECTORY } else { 0 };
            // SAFETY: parent is a live directory descriptor and name is terminated.
            let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return Err(error("PATH_UNAVAILABLE", path));
            }
            // SAFETY: fd was newly opened and is uniquely owned by this File.
            let next = unsafe { File::from_raw_fd(fd) };
            use std::os::unix::fs::MetadataExt;
            let actual = next.metadata().map_err(|_| error("READ_FAILED", path))?;
            // libc's dev_t/ino_t widths differ across supported Unix targets.
            #[allow(clippy::unnecessary_cast)]
            let expected_identity = (before.st_dev as u64, before.st_ino as u64);
            if (actual.dev(), actual.ino()) != expected_identity
                || if is_dir {
                    !actual.is_dir()
                } else {
                    !actual.is_file()
                }
            {
                return Err(error("FILESYSTEM_CHANGED", path));
            }
            parent = next;
        }
        Ok(parent)
    }

    pub fn names(file: File, limit: usize) -> Result<Vec<String>> {
        struct Directory(*mut libc::DIR);
        impl Drop for Directory {
            fn drop(&mut self) {
                unsafe {
                    libc::closedir(self.0);
                }
            }
        }
        let fd = file.into_raw_fd();
        // SAFETY: fd is an owned directory descriptor; fdopendir takes it on success.
        let raw = unsafe { libc::fdopendir(fd) };
        if raw.is_null() {
            // SAFETY: fdopendir failed and did not take ownership.
            unsafe {
                libc::close(fd);
            }
            return Err(error("READ_FAILED", "cannot list projection directory"));
        }
        let dir = Directory(raw);
        let mut names = Vec::new();
        loop {
            // POSIX readdir requires clearing errno to distinguish EOF from failure.
            errno::set_errno(errno::Errno(0));
            // SAFETY: dir stays live and this function is its exclusive reader.
            let entry = unsafe { libc::readdir(dir.0) };
            if entry.is_null() {
                if errno::errno().0 != 0 {
                    return Err(error("READ_FAILED", "cannot list projection directory"));
                }
                break;
            }
            // SAFETY: readdir returns a terminated d_name valid until the next call.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }
                .to_str()
                .map_err(|_| error("INVALID_UTF8", "projection directory entry"))?;
            if name == "." || name == ".." {
                continue;
            }
            if names.len() >= limit {
                return Err(error("LIMIT_EXCEEDED", "directory entry count"));
            }
            relative(name)?;
            names.push(name.to_owned());
        }
        names.sort();
        Ok(names)
    }
}

#[cfg(not(unix))]
mod confined {
    use super::*;
    fn unsupported() -> Error {
        error(
            "UNSUPPORTED_PLATFORM",
            "confined project inspection currently requires Unix",
        )
    }
    pub fn root(_: &Path) -> Result<File> {
        Err(unsupported())
    }
    pub fn open(_: &File, _: &str, _: bool) -> Result<File> {
        Err(unsupported())
    }
    pub fn names(_: File, _: usize) -> Result<Vec<String>> {
        Err(unsupported())
    }
}

fn check_graph(
    graph: &BTreeMap<String, Vec<String>>,
    depth_limit: usize,
    label: &str,
) -> Result<()> {
    fn visit<'a>(
        id: &'a str,
        graph: &'a BTreeMap<String, Vec<String>>,
        path: &mut BTreeSet<&'a str>,
        memo: &mut BTreeMap<&'a str, usize>,
        limit: usize,
        label: &str,
    ) -> Result<usize> {
        if path.contains(id) {
            return Err(error("DEPENDENCY_CYCLE", format!("{label}: cycle at {id}")));
        }
        if path.len() >= limit {
            return Err(error(
                "LIMIT_EXCEEDED",
                format!("{label}: dependency depth"),
            ));
        }
        if let Some(depth) = memo.get(id) {
            if path.len() + depth > limit {
                return Err(error(
                    "LIMIT_EXCEEDED",
                    format!("{label}: dependency depth"),
                ));
            }
            return Ok(*depth);
        }
        let deps = graph.get(id).ok_or_else(|| {
            error(
                "MISSING_REFERENCE",
                format!("{label}: unknown dependency {id}"),
            )
        })?;
        path.insert(id);
        let mut depth = 1;
        for dep in deps {
            depth = depth.max(1 + visit(dep, graph, path, memo, limit, label)?);
        }
        path.remove(id);
        memo.insert(id, depth);
        Ok(depth)
    }
    let mut memo = BTreeMap::new();
    for id in graph.keys() {
        visit(
            id,
            graph,
            &mut BTreeSet::new(),
            &mut memo,
            depth_limit,
            label,
        )?;
    }
    Ok(())
}

impl Project {
    /// Existing logical work IDs, for collision checks without exposing source text.
    pub fn work_ids(&self) -> impl Iterator<Item = &str> {
        self.work.keys().map(String::as_str)
    }

    pub fn project_id(&self) -> &str {
        &self.manifest.id
    }

    pub fn load(root: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_limits(root, Limits::default())
    }

    pub fn load_with_limits(root: impl AsRef<Path>, limits: Limits) -> Result<Self> {
        let mut reader = Reader::new(root.as_ref(), limits)?;
        let project_path = ".astral/project.toml";
        let manifest: ProjectManifest = schema(&reader.read(project_path)?, project_path)?;
        version(manifest.schema_version, project_path)?;
        identifier(&manifest.id)?;
        nonempty(&manifest.name, "project.name")?;
        nonempty(&manifest.description, "project.description")?;
        nonempty(&manifest.identity.scope, "identity.scope")?;
        nonempty(
            &manifest.identity.runtime_bindings,
            "identity.runtime_bindings",
        )?;
        let core = join(".astral", &manifest.core)?;
        let projection_root = join(".astral", &manifest.projections)?;
        let work_path = join(".astral", &manifest.work_items)?;
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            reader.read(&join(&core, name)?)?;
        }

        let mut work = BTreeMap::new();
        let raw_work = reader.read(&work_path)?;
        let text = std::str::from_utf8(&raw_work).map_err(|_| error("INVALID_UTF8", &work_path))?;
        for (line_number, line) in text.lines().enumerate() {
            reader.charge(1)?;
            // Blank/comment lines are malformed records, not silently skipped.
            let item: WorkItem = serde_json::from_str(line).map_err(|_| {
                error(
                    "INVALID_WORK_ITEM",
                    format!(
                        "{work_path}: invalid JSON/schema on line {}",
                        line_number + 1
                    ),
                )
            })?;
            version(item.schema_version, &work_path)?;
            identifier(&item.id)?;
            nonempty(&item.title, "work.title")?;
            distinct(&item.depends_on, "work.depends_on")?;
            distinct(&item.acceptance, "work.acceptance")?;
            if item.acceptance.is_empty() {
                return Err(error(
                    "INVALID_FIELD",
                    "work.acceptance must have at least one criterion",
                ));
            }
            reader.charge(
                item.depends_on.len()
                    + item.acceptance.len()
                    + item.evidence.as_ref().map_or(0, Vec::len),
            )?;
            for dep in &item.depends_on {
                identifier(dep)?;
            }
            if let Some(evidence) = &item.evidence {
                for path in evidence {
                    relative(path)?;
                }
            }
            let id = item.id.clone();
            let source = SourceHandle {
                path: work_path.clone(),
                sha256: crate::hash(line.as_bytes()),
                bytes: line.len(),
                record_id: Some(id.clone()),
            };
            if work
                .insert(
                    id.clone(),
                    WorkRecord {
                        item,
                        source,
                        text: line.to_owned(),
                    },
                )
                .is_some()
            {
                return Err(error("DUPLICATE_ID", format!("duplicate work item {id}")));
            }
        }
        check_graph(
            &work
                .iter()
                .map(|(id, record)| (id.clone(), record.item.depends_on.clone()))
                .collect(),
            limits.graph_depth,
            "work",
        )?;

        reader.charge(manifest.subsystems.len())?;
        let mut subsystems = BTreeMap::new();
        for (id, location) in &manifest.subsystems {
            identifier(id)?;
            let directory = join(".astral", location)?;
            let path = join(&directory, "subsystem.toml")?;
            let m: SubsystemManifest = schema(&reader.read(&path)?, &path)?;
            version(m.schema_version, &path)?;
            if &m.id != id {
                return Err(error(
                    "REGISTRY_MISMATCH",
                    format!("{path}: id must equal registry key {id}"),
                ));
            }
            nonempty(&m.purpose, "subsystem.purpose")?;
            identifier(&m.projection)?;
            distinct(&m.rules, "subsystem.rules")?;
            distinct(&m.decisions, "subsystem.decisions")?;
            distinct(&m.work_items, "subsystem.work_items")?;
            distinct(&m.depends_on, "subsystem.depends_on")?;
            reader.charge(
                m.rules.len() + m.decisions.len() + m.work_items.len() + m.depends_on.len(),
            )?;
            for dep in &m.depends_on {
                identifier(dep)?;
            }
            for id in &m.work_items {
                if !work.contains_key(id) {
                    return Err(error(
                        "MISSING_REFERENCE",
                        format!("{path}: unknown work item {id}"),
                    ));
                }
            }
            for doc in std::iter::once(&m.readme)
                .chain(m.rules.iter())
                .chain(m.decisions.iter())
            {
                reader.read(&join(&directory, doc)?)?;
            }
            subsystems.insert(
                id.clone(),
                Subsystem {
                    manifest: m,
                    directory,
                },
            );
        }
        check_graph(
            &subsystems
                .iter()
                .map(|(id, subsystem)| (id.clone(), subsystem.manifest.depends_on.clone()))
                .collect(),
            limits.graph_depth,
            "subsystems",
        )?;

        let mut projections = BTreeMap::new();
        for id in reader.directory(&projection_root)? {
            identifier(&id)?;
            let directory = join(&projection_root, &id)?;
            let path = join(&directory, "projection.toml")?;
            let m: ProjectionManifest = schema(&reader.read(&path)?, &path)?;
            version(m.schema_version, &path)?;
            if m.id != id {
                return Err(error(
                    "REGISTRY_MISMATCH",
                    format!("{path}: id must equal directory name {id}"),
                ));
            }
            nonempty(&m.kind, "projection.kind")?;
            let native = load_native_bundle(&mut reader, &directory, &m, &manifest.id)?;
            distinct(&m.subsystems, "projection.subsystems")?;
            reader.charge(m.subsystems.len() + m.sources.len())?;
            for id in &m.subsystems {
                if !subsystems.contains_key(id) {
                    return Err(error(
                        "MISSING_REFERENCE",
                        format!("{path}: unknown subsystem {id}"),
                    ));
                }
            }
            let mut source_ids = BTreeSet::new();
            for source in &m.sources {
                identifier(&source.id)?;
                if !source_ids.insert(&source.id) {
                    return Err(error(
                        "DUPLICATE_ID",
                        format!("{path}: duplicate provenance id {}", source.id),
                    ));
                }
                nonempty(&source.kind, "source.kind")?;
                nonempty(&source.scope, "source.scope")?;
                nonempty(&source.availability, "source.availability")?;
            }
            reader.read(&join(&directory, &m.handoff)?)?;
            projections.insert(
                id,
                Projection {
                    manifest: m,
                    directory,
                    native,
                },
            );
        }
        for subsystem in subsystems.values() {
            if !projections.contains_key(&subsystem.manifest.projection) {
                return Err(error(
                    "MISSING_REFERENCE",
                    format!(
                        "subsystem {}: unknown projection {}",
                        subsystem.manifest.id, subsystem.manifest.projection
                    ),
                ));
            }
        }
        validate_native_document_roles(&reader, &core, &subsystems, &projections)?;
        let contents = reader.contents;
        let sources = contents
            .iter()
            .map(|(path, bytes)| {
                let handle = SourceHandle {
                    path: path.clone(),
                    sha256: crate::hash(bytes),
                    bytes: bytes.len(),
                    record_id: None,
                };
                (path.clone(), handle)
            })
            .collect();
        Ok(Self {
            manifest,
            subsystems,
            projections,
            work,
            sources,
            contents,
            limits,
        })
    }

    fn output(&self, value: Value) -> Result<Value> {
        if serde_json::to_vec(&value).expect("Value serializes").len() > self.limits.output_bytes {
            return Err(error("LIMIT_EXCEEDED", "output byte limit"));
        }
        Ok(value)
    }

    pub fn validate(&self) -> Result<Value> {
        self.output(json!({
            "schema_version": 1, "status": "valid", "project_id": self.manifest.id,
            "subsystems": self.subsystems.len(), "projections": self.projections.len(),
            "work_items": self.work.len(), "observed_files": self.sources.len(),
            "native_artifacts": self.projections.values().filter(|p| p.native.is_some()).count(),
            "native_binding": native_binding(),
            "verification_scope": "Declared local inputs only; per-file observations, not an atomic tree snapshot, native recall, or execution verification"
        }))
    }

    pub fn list(&self) -> Result<Value> {
        let mut contexts = Vec::new();
        for id in self.subsystems.keys() {
            contexts.push(
                json!({"id": id, "kind": "subsystem", "selector": format!("subsystem:{id}")}),
            );
        }
        for id in self.projections.keys() {
            contexts.push(
                json!({"id": id, "kind": "projection", "selector": format!("projection:{id}")}),
            );
        }
        self.output(json!({"schema_version": 1, "project_id": self.manifest.id, "contexts": contexts, "native_binding": native_binding()}))
    }

    fn resolve(&self, selector: &str, work_id: Option<&str>) -> Result<ResolvedSelection<'_>> {
        let (kind, id) = if let Some((kind, id)) = selector.split_once(':') {
            if kind != "subsystem" && kind != "projection" {
                return Err(error(
                    "INVALID_SELECTOR",
                    "use subsystem:NAME or projection:NAME",
                ));
            }
            (kind, id)
        } else {
            match (
                self.subsystems.contains_key(selector),
                self.projections.contains_key(selector),
            ) {
                (true, true) => {
                    return Err(error(
                        "AMBIGUOUS_CONTEXT",
                        format!("{selector}: use subsystem:{selector} or projection:{selector}"),
                    ));
                }
                (true, false) => ("subsystem", selector),
                (false, true) => ("projection", selector),
                (false, false) => return Err(error("UNKNOWN_CONTEXT", selector)),
            }
        };
        identifier(id)?;
        let mut selected = BTreeSet::new();
        let projection = if kind == "projection" {
            let projection = self
                .projections
                .get(id)
                .ok_or_else(|| error("UNKNOWN_CONTEXT", selector))?;
            for id in &projection.manifest.subsystems {
                self.closure(id, &mut selected);
            }
            Some(projection)
        } else {
            if !self.subsystems.contains_key(id) {
                return Err(error("UNKNOWN_CONTEXT", selector));
            }
            self.closure(id, &mut selected);
            None
        };
        let work = work_id
            .map(|id| {
                self.work
                    .get(id)
                    .ok_or_else(|| error("UNKNOWN_WORK_ITEM", id))
            })
            .transpose()?;
        let mut paths = BTreeSet::from([".astral/project.toml".to_owned()]);
        let mut document_paths = BTreeSet::new();
        let core = join(".astral", &self.manifest.core)?;
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            document_paths.insert(join(&core, name)?);
        }
        for id in &selected {
            let subsystem = &self.subsystems[id];
            let m = &subsystem.manifest;
            paths.insert(join(&subsystem.directory, "subsystem.toml")?);
            for doc in std::iter::once(&m.readme)
                .chain(m.rules.iter())
                .chain(m.decisions.iter())
            {
                document_paths.insert(join(&subsystem.directory, doc)?);
            }
        }
        if let Some(projection) = projection {
            paths.insert(join(&projection.directory, "projection.toml")?);
            document_paths.insert(join(&projection.directory, &projection.manifest.handoff)?);
        }
        // Native artifacts are part of a selected subsystem's saved context as
        // well as an explicit projection. Include every effective native source
        // in the fingerprint, without treating its payload as a readable document.
        for projection in projection.into_iter().chain(
            selected
                .iter()
                .map(|id| &self.projections[&self.subsystems[id].manifest.projection]),
        ) {
            if let Some(native) = &projection.native {
                paths.insert(join(&projection.directory, "projection.toml")?);
                paths.insert(native.manifest_path.clone());
                paths.insert(native.payload_path.clone());
            }
        }
        paths.extend(document_paths.iter().cloned());
        let mut sources: Vec<_> = paths
            .iter()
            .map(|path| self.sources[path].clone())
            .collect();
        if let Some(work) = work {
            sources.push(work.source.clone());
        }
        sources.sort_by(|a, b| (&a.path, &a.record_id).cmp(&(&b.path, &b.record_id)));
        let selection = ContextSelection {
            kind: if kind == "projection" {
                ContextKind::Projection
            } else {
                ContextKind::Subsystem
            },
            id: id.to_owned(),
            subsystems: selected.into_iter().collect(),
            projection: projection.map(|p| p.manifest.id.clone()),
            work_item: work_id.map(str::to_owned),
        };
        let selection_digest = crate::fingerprint(
            &json!({"schema_version": 1, "selection": selection, "sources": sources}),
        );
        Ok(ResolvedSelection {
            selection,
            selection_digest,
            sources,
            document_paths,
            projection,
            work,
        })
    }

    pub fn inspect(&self, selector: &str, work_id: Option<&str>) -> Result<Value> {
        let resolved = self.resolve(selector, work_id)?;
        let subsystem_metadata: BTreeMap<_, _> = resolved
            .selection
            .subsystems
            .iter()
            .map(|id| (id, &self.subsystems[id].manifest))
            .collect();
        self.output(json!({
            "schema_version": 1, "mode": "inspect", "project_id": self.manifest.id,
            "selection": resolved.selection, "sources": resolved.sources, "selection_digest": resolved.selection_digest,
            "fingerprint_scope": "Selected declared source bytes and exact selected JSONL record; no machine path, time, provider semantics, or atomic snapshot claim",
            "metadata": {"project": self.manifest, "subsystems": subsystem_metadata, "projection": resolved.projection.map(|p| &p.manifest)},
            "work_item": resolved.work.map(|w| &w.item), "native_binding": native_binding(),
            "native_artifacts": self.selected_native_artifacts(&resolved)
        }))
    }

    /// Metadata only: no payload, instructions, credentials or executable settings.
    pub fn native_artifacts(
        &self,
        selector: &str,
        work_id: Option<&str>,
    ) -> Result<Vec<NativeArtifact>> {
        let resolved = self.resolve(selector, work_id)?;
        let artifacts = self.selected_native_artifacts(&resolved);
        if serde_json::to_vec(&artifacts)
            .expect("artifact metadata serializes")
            .len()
            > self.limits.output_bytes
        {
            return Err(error(
                "LIMIT_EXCEEDED",
                "native artifact metadata output byte limit",
            ));
        }
        Ok(artifacts)
    }

    fn selected_native_artifacts(&self, resolved: &ResolvedSelection<'_>) -> Vec<NativeArtifact> {
        let mut artifacts = BTreeMap::new();
        for projection in resolved.projection.into_iter().chain(
            resolved
                .selection
                .subsystems
                .iter()
                .map(|id| &self.projections[&self.subsystems[id].manifest.projection]),
        ) {
            if let Some(native) = &projection.native {
                artifacts
                    .entry(projection.manifest.id.clone())
                    .or_insert_with(|| NativeArtifact {
                        projection: projection.manifest.id.clone(),
                        availability: NativeAvailability::Validated,
                        runtime_binding: NativeRuntimeBinding::Unbound,
                        manifest: self.sources[&native.manifest_path].clone(),
                        payload: self.sources[&native.payload_path].clone(),
                        bundle: native.bundle.summary(),
                    });
            }
        }
        artifacts.into_values().collect()
    }

    /// Assemble cached readable context for a fresh session. Explicit and linked
    /// projections must be `reviewable-design-context`, `reviewable`, or
    /// `fresh-context`; native and unknown kinds never become plaintext fallback.
    /// Provenance descriptions remain inert. This is neither a native restore nor
    /// an atomic workspace snapshot, and the usual serialized output limit applies.
    pub fn fresh_context(&self, selector: &str, work_id: Option<&str>) -> Result<FreshContext> {
        self.build_launch_context(selector, work_id, false)
            .map(|context| context.current_context)
    }

    pub fn launch_context(
        &self,
        selector: &str,
        work_id: Option<&str>,
    ) -> Result<ProjectLaunchContext> {
        self.build_launch_context(selector, work_id, true)
    }

    fn build_launch_context(
        &self,
        selector: &str,
        work_id: Option<&str>,
        allow_native: bool,
    ) -> Result<ProjectLaunchContext> {
        let resolved = self.resolve(selector, work_id)?;
        let mut native_bundles = BTreeMap::new();
        // An explicitly selected native projection is the chosen conversation.
        // Linked subsystems still supply current documents, not additional seeds.
        let explicit_native = resolved.projection.and_then(|p| p.native.as_ref());
        let mut fresh_sources: BTreeMap<_, _> = resolved
            .sources
            .iter()
            .map(|source| {
                (
                    (source.path.clone(), source.record_id.clone()),
                    source.clone(),
                )
            })
            .collect();
        for projection in resolved.projection.into_iter().chain(
            resolved
                .selection
                .subsystems
                .iter()
                .map(|id| &self.projections[&self.subsystems[id].manifest.projection]),
        ) {
            if let Some(native) = &projection.native {
                if !allow_native {
                    return Err(error(
                        "NATIVE_CONTEXT_REQUIRES_STAGING",
                        format!(
                            "projection {} requires native staging; it cannot launch as fresh document context",
                            projection.manifest.id
                        ),
                    ));
                }
                if explicit_native.is_none_or(|chosen| {
                    chosen.bundle.summary().manifest_sha256
                        == native.bundle.summary().manifest_sha256
                }) {
                    native_bundles.insert(
                        native.bundle.summary().manifest_sha256.clone(),
                        &native.bundle,
                    );
                }
            } else if !matches!(
                projection.manifest.kind.as_str(),
                "reviewable-design-context" | "reviewable" | "fresh-context"
            ) {
                return Err(error(
                    "UNSUPPORTED_FRESH_PROJECTION",
                    format!(
                        "projection {} has unsupported fresh-context kind {:?}",
                        projection.manifest.id, projection.manifest.kind
                    ),
                ));
            }
            let path = join(&projection.directory, "projection.toml")?;
            fresh_sources.insert((path.clone(), None), self.sources[&path].clone());
        }
        if native_bundles.len() > 1 {
            return Err(error(
                "MULTIPLE_NATIVE_CONTEXTS",
                "selection contains distinct native histories; choose one projection instead of implicitly merging them",
            ));
        }
        let mut text_bytes = resolved.work.map_or(0, |work| work.text.len());
        let mut documents = Vec::new();
        for path in &resolved.document_paths {
            let bytes = &self.contents[path];
            text_bytes = text_bytes.saturating_add(bytes.len());
            if text_bytes > self.limits.output_bytes {
                return Err(error("LIMIT_EXCEEDED", "fresh context text byte limit"));
            }
            let text = std::str::from_utf8(bytes)
                .map_err(|_| error("INVALID_UTF8", path))?
                .to_owned();
            documents.push(FreshDocument {
                source: self.sources[path].clone(),
                text,
            });
        }
        let sources: Vec<_> = fresh_sources.into_values().collect();
        let selection_digest = crate::fingerprint(
            &json!({"schema_version": 1, "selection": resolved.selection, "sources": sources}),
        );
        let context = FreshContext {
            schema_version: 1,
            project_id: self.manifest.id.clone(),
            selection: resolved.selection,
            selection_digest,
            sources,
            documents,
            work_item: resolved.work.map(|work| FreshWorkItem {
                item: work.item.clone(),
                source: work.source.clone(),
                text: work.text.clone(),
            }),
        };
        if serde_json::to_vec(&context)
            .expect("fresh context serializes")
            .len()
            > self.limits.output_bytes
        {
            return Err(error("LIMIT_EXCEEDED", "fresh context output byte limit"));
        }
        Ok(ProjectLaunchContext {
            current_context: context,
            native: native_bundles.into_values().next().cloned(),
        })
    }

    fn closure(&self, id: &str, selected: &mut BTreeSet<String>) {
        if selected.insert(id.to_owned()) {
            for dep in &self.subsystems[id].manifest.depends_on {
                self.closure(dep, selected);
            }
        }
    }
}

fn native_binding() -> Value {
    json!({"state": "UNBOUND", "launch": false, "plaintext_substitution": false,
        "reason": "Inspection does not bind a destination runtime. Native launch requires explicit proxy routing on the supported Codex route."})
}
