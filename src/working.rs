//! Host-owned working state. Native reasoning/compaction items never enter this reducer.
use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;

const MAX_BLOB: usize = 8 * 1024 * 1024;
const MAX_JOURNAL: u64 = 32 * 1024 * 1024;
const MAX_PAGE: usize = 16 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackedFile {
    pub path: String,
    #[serde(default)]
    pub writable: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub argv: Vec<String>,
    pub timeout_seconds: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub files: Vec<TrackedFile>,
    #[serde(default)]
    pub checks: BTreeMap<String, Check>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Version {
    sha256: Option<String>,
    generation: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Event {
    sequence: u64,
    kind: String,
    value: Value,
}
#[derive(Default)]
struct State {
    files: BTreeMap<String, Version>,
    checks: BTreeMap<String, Value>,
    constraints: BTreeMap<String, Value>,
    obligations: BTreeMap<String, Value>,
    actions: BTreeMap<String, Value>,
}
impl State {
    fn apply(&mut self, event: &Event) -> anyhow::Result<()> {
        match event.kind.as_str() {
            "files" => self.files = serde_json::from_value(event.value.clone())?,
            "check" => {
                self.checks
                    .insert(field(&event.value, "name")?.into(), event.value.clone());
            }
            "constraint" => {
                self.constraints
                    .insert(field(&event.value, "id")?.into(), event.value.clone());
            }
            "obligation" => {
                self.obligations
                    .insert(field(&event.value, "id")?.into(), event.value.clone());
            }
            "action" => {
                self.actions
                    .insert(field(&event.value, "id")?.into(), event.value.clone());
            }
            "binding" => {}
            _ => bail!("unknown journal event"),
        }
        Ok(())
    }
}

pub struct Runtime {
    root: PathBuf,
    dir: PathBuf,
    profile: Profile,
    state: State,
    events: Vec<Event>,
    journal: File,
    _lock: File,
    compact: bool,
}
fn field<'a>(v: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .with_context(|| format!("missing string field: {key}"))
}
fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
fn valid_handle(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}
fn sync_dir(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
fn atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let parent = path.parent().context("missing parent")?;
    let tmp = parent.join(format!(
        ".astral-{}-{}.tmp",
        std::process::id(),
        crate::hash(bytes)
    ));
    // Single writer owns the runtime directory. Temporary files never become artifacts.
    let mut file = OpenOptions::new().create_new(true).write(true).open(&tmp)?;
    if let Ok(metadata) = std::fs::metadata(path) {
        file.set_permissions(metadata.permissions())?;
    }
    let result = (|| -> anyhow::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        sync_dir(parent)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(tmp);
    }
    result
}

impl Runtime {
    pub fn open(root: &Path, dir: &Path, profile: Profile, compact: bool) -> anyhow::Result<Self> {
        let root = root.canonicalize()?;
        ensure!(root.is_dir(), "workspace must be a directory");
        ensure!(
            !profile.files.is_empty() && profile.files.len() <= 512,
            "track 1..512 explicit files"
        );
        let mut unique = std::collections::HashSet::new();
        for f in &profile.files {
            ensure!(
                safe_relative(&f.path) && unique.insert(&f.path),
                "invalid or duplicate tracked path"
            );
        }
        for c in profile.checks.values() {
            ensure!(
                !c.argv.is_empty() && c.timeout_seconds > 0 && c.timeout_seconds <= 600,
                "invalid check command or timeout"
            );
        }
        std::fs::create_dir_all(dir)?;
        let dir = dir.canonicalize()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        std::fs::create_dir_all(dir.join("blobs"))?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(dir.join("process.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock).context("another host owns working state")?;
        let mut journal = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .append(true)
            .open(dir.join("events.jsonl"))?;
        ensure!(
            journal.metadata()?.len() <= MAX_JOURNAL,
            "working journal limit exceeded"
        );
        let mut bytes = Vec::new();
        journal.read_to_end(&mut bytes)?;
        // A torn final append is uncommitted. Complete malformed records fail closed.
        let end = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |n| n + 1);
        if end != bytes.len() {
            journal.set_len(end as u64)?;
            journal.sync_all()?;
            bytes.truncate(end);
        }
        let mut state = State::default();
        let mut events = Vec::new();
        for line in bytes.split(|b| *b == b'\n').filter(|b| !b.is_empty()) {
            let event: Event = serde_json::from_slice(line).context("corrupt working journal")?;
            ensure!(
                event.sequence == events.len() as u64 + 1,
                "journal sequence mismatch"
            );
            state.apply(&event)?;
            events.push(event);
        }
        let binding = json!({"workspace":root,"profile":profile,"schema":1});
        if let Some(first) = events.first() {
            ensure!(
                first.kind == "binding" && first.value == binding,
                "workspace/profile changed; use a separate state directory"
            );
        }
        let mut r = Self {
            root,
            dir,
            profile,
            state,
            events,
            journal,
            _lock: lock,
            compact,
        };
        if r.events.is_empty() {
            r.commit("binding", binding)?;
        }
        r.refresh()?;
        Ok(r)
    }
    fn commit(&mut self, kind: &str, value: Value) -> anyhow::Result<()> {
        let event = Event {
            sequence: self.events.len() as u64 + 1,
            kind: kind.into(),
            value,
        };
        let mut bytes = serde_json::to_vec(&event)?;
        bytes.push(b'\n');
        ensure!(
            self.journal.metadata()?.len() + bytes.len() as u64 <= MAX_JOURNAL,
            "working journal full; no event committed"
        );
        self.journal.write_all(&bytes)?;
        self.journal.sync_all()?;
        self.state.apply(&event)?;
        self.events.push(event);
        Ok(())
    }
    fn path(&self, name: &str, writing: bool) -> anyhow::Result<PathBuf> {
        let tracked = self
            .profile
            .files
            .iter()
            .find(|f| f.path == name)
            .context("file is not tracked")?;
        ensure!(!writing || tracked.writable, "file is read-only");
        let path = self.root.join(name);
        let mut prefix = self.root.clone();
        for component in Path::new(name).components() {
            prefix.push(component);
            match std::fs::symlink_metadata(&prefix) {
                Ok(m) => ensure!(
                    !m.file_type().is_symlink(),
                    "symlink paths are not supported"
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        ensure!(
            !path.starts_with(&self.dir),
            "state files cannot be tracked"
        );
        Ok(path)
    }
    fn read_bounded(path: &Path) -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        File::open(path)?
            .take((MAX_BLOB + 1) as u64)
            .read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_BLOB, "artifact exceeds 8 MiB limit");
        Ok(bytes)
    }
    fn refresh(&mut self) -> anyhow::Result<()> {
        let mut files = BTreeMap::new();
        for tracked in &self.profile.files {
            let path = self.path(&tracked.path, false)?;
            let sha256 = match Self::read_bounded(&path) {
                Ok(bytes) => Some(crate::hash(&bytes)),
                Err(e)
                    if e.downcast_ref::<std::io::Error>()
                        .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                {
                    None
                }
                Err(e) => return Err(e),
            };
            let previous = self.state.files.get(&tracked.path);
            let generation = previous.map_or(1, |v| v.generation + u64::from(v.sha256 != sha256));
            files.insert(tracked.path.clone(), Version { sha256, generation });
        }
        if files != self.state.files {
            self.commit("files", serde_json::to_value(files)?)?;
        }
        Ok(())
    }
    fn input_digest(&self) -> String {
        crate::fingerprint(&json!({"files":self.state.files,"checks":self.profile.checks}))
    }
    fn blob(&self, bytes: &[u8]) -> anyhow::Result<String> {
        ensure!(bytes.len() <= MAX_BLOB, "artifact exceeds 8 MiB limit");
        let handle = crate::hash(bytes);
        let path = self.dir.join("blobs").join(&handle);
        if path.exists() {
            ensure!(
                Self::read_bounded(&path)? == bytes,
                "artifact integrity failure"
            );
        } else {
            atomic_write(&path, bytes)?;
        }
        Ok(handle)
    }
    fn load_blob(&self, handle: &str) -> anyhow::Result<Vec<u8>> {
        ensure!(valid_handle(handle), "invalid artifact handle");
        let bytes = Self::read_bounded(&self.dir.join("blobs").join(handle))?;
        ensure!(crate::hash(&bytes) == handle, "artifact integrity failure");
        Ok(bytes)
    }
    fn snapshot(&self) -> Value {
        let digest = self.input_digest();
        let checks: BTreeMap<_, _> = self
            .state
            .checks
            .iter()
            .map(|(name, check)| {
                let mut check = check.clone();
                check["current"] =
                    json!(check["input_digest"] == digest && check["stable_during_run"] == true);
                (name, check)
            })
            .collect();
        json!({"schema":"astral.state.v1","sequence":self.events.len(),"input_digest":digest,
            "files":self.state.files,"constraints":self.state.constraints,"checks":checks,"obligations":self.state.obligations})
    }
    pub async fn dispatch(&mut self, request: &Value) -> anyhow::Result<Value> {
        let Some(id) = request.get("action_id").and_then(Value::as_str) else {
            return self.execute(request).await;
        };
        ensure!(!id.is_empty() && id.len() <= 256, "invalid action ID");
        let request_hash = crate::fingerprint(request);
        if let Some(action) = self.state.actions.get(id) {
            ensure!(
                action["request_sha256"] == request_hash,
                "action ID reused with different arguments"
            );
            match action["status"].as_str() {
                Some("completed") => {
                    return Ok(serde_json::from_slice(
                        &self.load_blob(field(action, "handle")?)?,
                    )?);
                }
                Some("failed") => bail!("{}", field(action, "error")?),
                _ => bail!(
                    "action outcome uncertain after interruption; inspect state before issuing a new action ID"
                ),
            }
        }
        self.commit(
            "action",
            json!({"id":id,"status":"started","request_sha256":request_hash}),
        )?;
        match self.execute(request).await {
            Ok(mut result) => {
                // The response names the sequence at which it becomes durable.
                result["sequence"] = json!(self.events.len() + 1);
                let handle = self.blob(&serde_json::to_vec(&result)?)?;
                self.commit("action", json!({"id":id,"status":"completed","request_sha256":request_hash,"handle":handle}))?;
                Ok(result)
            }
            Err(error) => {
                self.commit("action", json!({"id":id,"status":"failed","request_sha256":request_hash,"error":error.to_string()}))?;
                Err(error)
            }
        }
    }
    async fn execute(&mut self, request: &Value) -> anyhow::Result<Value> {
        let before = self.events.len();
        self.refresh()?;
        let mut result = match field(request, "op")? {
            "snapshot" => self.snapshot(),
            "check_history" => {
                let name = field(request, "name")?;
                ensure!(
                    self.profile.checks.contains_key(name),
                    "check is not configured"
                );
                let before = request["before_sequence"]
                    .as_u64()
                    .filter(|v| *v > 0)
                    .unwrap_or(u64::MAX);
                let receipts: Vec<_> = self.events.iter().rev().filter(|e| e.kind == "check" && e.value["name"] == name && e.sequence < before).take(20)
                    .map(|e| json!({"sequence":e.sequence,"name":name,"passed":e.value["passed"],"input_digest":e.value["input_digest"],"stdout_handle":e.value["stdout_handle"],"stderr_handle":e.value["stderr_handle"]})).collect();
                json!({"next_before_sequence":receipts.last().map(|r|r["sequence"].clone()),"receipts":receipts})
            }
            "constraint" | "obligation" => {
                let op = field(request, "op")?;
                let id = field(request, "id")?;
                let text = field(request, "text")?;
                ensure!(
                    !id.is_empty() && id.len() <= 128 && text.len() <= 8192,
                    "invalid state entry size"
                );
                if let Some(expected) = request.get("expected_sequence") {
                    ensure!(
                        expected.as_u64()
                            == Some(
                                self.events.len() as u64
                                    - u64::from(
                                        request.get("action_id").and_then(Value::as_str).is_some()
                                    )
                            ),
                        "state compare-and-swap conflict"
                    );
                }
                let source = field(request, "source")?;
                ensure!(source.len() <= 8192, "source too large");
                self.commit(op, json!({"id":id,"text":text,"source":source,"status":request.get("status").cloned().unwrap_or(json!("open"))}))?;
                json!({"recorded":true})
            }
            "read_file" => {
                let name = field(request, "path")?;
                let bytes = Self::read_bounded(&self.path(name, false)?)?;
                let handle = self.blob(&bytes)?;
                let text = std::str::from_utf8(&bytes)
                    .context("use artifact retrieval for non-UTF8 files")?;
                let start = request["start_line"].as_u64().unwrap_or(1) as usize;
                let count = request["lines"].as_u64().unwrap_or(120) as usize;
                ensure!(
                    start > 0 && count > 0 && count <= 200,
                    "line range must be positive and at most 200 lines"
                );
                let selected = if self.compact {
                    text.lines()
                        .skip(start - 1)
                        .take(count)
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    text.into()
                };
                ensure!(
                    selected.len() <= if self.compact { MAX_PAGE } else { MAX_BLOB },
                    "requested page too large; use byte retrieval"
                );
                json!({"path":name,"version":self.state.files[name],"handle":handle,"total_bytes":bytes.len(),"total_lines":text.lines().count(),"start_line":if self.compact {start} else {1},"text":selected})
            }
            "write_file" => {
                let name = field(request, "path")?;
                let path = self.path(name, true)?;
                let expected = request
                    .get("expected_sha256")
                    .context("expected_sha256 is required (null for absent file)")?;
                ensure!(
                    expected == &json!(self.state.files[name].sha256),
                    "file compare-and-swap conflict"
                );
                let content = field(request, "content")?.as_bytes();
                ensure!(content.len() <= MAX_BLOB, "file too large");
                if path.exists() {
                    self.blob(&Self::read_bounded(&path)?)?;
                }
                let handle = self.blob(content)?;
                atomic_write(&path, content)?;
                self.refresh()?;
                json!({"path":name,"version":self.state.files[name],"handle":handle,"written":true})
            }
            "recall" => {
                let handle = field(request, "handle")?;
                let bytes = self.load_blob(handle)?;
                let offset = request["offset"].as_u64().unwrap_or(0) as usize;
                let limit = request["limit"].as_u64().unwrap_or(4096) as usize;
                ensure!(
                    offset <= bytes.len() && limit > 0 && limit <= MAX_PAGE,
                    "invalid byte range"
                );
                let end = offset.saturating_add(limit).min(bytes.len());
                let slice = &bytes[offset..end];
                let mut result = json!({"handle":handle,"offset":offset,"next_offset":end,"total_bytes":bytes.len(),"eof":end==bytes.len()});
                match std::str::from_utf8(slice) {
                    Ok(text) => {
                        result["encoding"] = json!("utf8");
                        result["data"] = json!(text);
                    }
                    Err(_) => {
                        result["encoding"] = json!("hex");
                        result["data"] =
                            json!(slice.iter().map(|b| format!("{b:02x}")).collect::<String>());
                    }
                }
                result
            }
            "search" => {
                let handle = field(request, "handle")?;
                let bytes = self.load_blob(handle)?;
                let text = std::str::from_utf8(&bytes)?;
                let query = field(request, "query")?;
                ensure!(
                    !query.is_empty() && query.len() <= 256,
                    "query must be 1..256 bytes"
                );
                let hits: Vec<_> = text
                    .match_indices(query)
                    .take(20)
                    .map(|(offset, matched)| json!({"offset":offset,"length":matched.len()}))
                    .collect();
                json!({"handle":handle,"matches":hits,"more_possible":hits.len()==20,"total_bytes":bytes.len()})
            }
            "check" => self.check(field(request, "name")?).await?,
            "roll_estimate" => {
                let estimate: crate::economics::RollEstimate =
                    serde_json::from_value(request["estimate"].clone())?;
                let net = estimate.net_savings()?;
                json!({"estimated_net_savings":net,"roll":net>0.0,"basis":"caller_estimates"})
            }
            _ => bail!("unknown host operation"),
        };
        result["sequence"] = json!(self.events.len());
        result["updates"] = serde_json::to_value(
            self.events[before..]
                .iter()
                .filter(|e| !matches!(e.kind.as_str(), "binding" | "action"))
                .collect::<Vec<_>>(),
        )?;
        Ok(result)
    }
    async fn check(&mut self, name: &str) -> anyhow::Result<Value> {
        let check = self
            .profile
            .checks
            .get(name)
            .context("check is not configured")?
            .clone();
        let input_digest = self.input_digest();
        let mut command = tokio::process::Command::new(&check.argv[0]);
        command
            .args(&check.argv[1..])
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().context("cannot start configured check")?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        async fn capture<R: tokio::io::AsyncRead + Unpin>(reader: R) -> std::io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            reader
                .take((MAX_BLOB + 1) as u64)
                .read_to_end(&mut bytes)
                .await?;
            Ok(bytes)
        }
        let result = tokio::time::timeout(Duration::from_secs(check.timeout_seconds), async {
            tokio::join!(child.wait(), capture(stdout), capture(stderr))
        })
        .await;
        let (exit, stdout, stderr, timed_out) = match result {
            Ok((exit, out, err)) => (exit?.code(), out?, err?, false),
            Err(_) => {
                let _ = child.kill().await;
                (None, Vec::new(), Vec::new(), true)
            }
        };
        let overflow = stdout.len() > MAX_BLOB || stderr.len() > MAX_BLOB;
        let out_handle = self.blob(&stdout[..stdout.len().min(MAX_BLOB)])?;
        let err_handle = self.blob(&stderr[..stderr.len().min(MAX_BLOB)])?;
        let parsed = check_summary(&stdout);
        self.refresh()?;
        let stable = input_digest == self.input_digest();
        let passed = !timed_out
            && !overflow
            && exit == Some(0)
            && parsed
                .as_ref()
                .is_some_and(|v| v["failed"] == 0 && v["passed"].as_u64().is_some_and(|n| n > 0));
        let receipt = json!({"name":name,"input_digest":input_digest,"stable_during_run":stable,
            "command_sha256":crate::fingerprint(&json!(check.argv)),"exit_code":exit,"timed_out":timed_out,"truncated":overflow,"capture_complete":!timed_out && !overflow,
            "passed":passed,"summary":parsed,"stdout_handle":out_handle,"stderr_handle":err_handle,"stdout_bytes":stdout.len(),"stderr_bytes":stderr.len()});
        self.commit("check", receipt.clone())?;
        let mut observation = receipt;
        if !self.compact || observation["summary"].is_null() || !passed {
            let limit = if self.compact { 4096 } else { MAX_BLOB };
            observation["stdout_preview"] =
                json!(String::from_utf8_lossy(&stdout[..stdout.len().min(limit)]));
            observation["stderr_preview"] =
                json!(String::from_utf8_lossy(&stderr[..stderr.len().min(limit)]));
        }
        Ok(observation)
    }
}

/// A narrow, documented adapter. Extra report fields remain available in the raw artifact.
pub fn check_summary(bytes: &[u8]) -> Option<Value> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    if v["schema"] != "astral.check.v1" {
        return None;
    }
    let passed = v["passed"].as_u64()?;
    let failed = v["failed"].as_u64()?;
    let failures = v["failures"].as_array()?;
    let warnings = v["warnings"].as_array()?;
    if failures.len() as u64 != failed || failures.len() > 100 || warnings.len() > 100 {
        return None;
    }
    let summary = json!({"passed":passed,"failed":failed,"failures":failures,"warnings":warnings,"available_fields":v.as_object()?.keys().collect::<Vec<_>>()});
    (serde_json::to_vec(&summary).ok()?.len() <= MAX_PAGE).then_some(summary)
}
