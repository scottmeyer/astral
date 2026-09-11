//! Private, bounded hook registration and best-effort notification metadata.
use crate::project::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) const MAX_BYTES: usize = 1_048_576;
pub(crate) fn error(code: &'static str, message: &str) -> Error {
    Error {
        code,
        message: message.into(),
    }
}
#[cfg(unix)]
fn name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 160
        || value == "."
        || value == ".."
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        return Err(error("HOOK_STORAGE_PATH", "invalid private metadata name"));
    }
    Ok(())
}
pub(crate) fn git_path(root: &Path, args: &[&str]) -> Result<PathBuf> {
    let bytes = crate::workspace::git_read(root, &args.iter().map(Into::into).collect::<Vec<_>>())?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| error("HOOK_PATH", "Git path is not UTF-8"))?
        .trim_end_matches('\n');
    if text.len() > 4096 || !Path::new(text).is_absolute() || text.contains('\0') {
        return Err(error("HOOK_PATH", "Git path must be bounded and absolute"));
    }
    Ok(text.into())
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct Snapshot {
    pub bytes: Vec<u8>,
    pub sha256: String,
    pub identity: String,
    pub mode: u32,
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub(crate) use unix::{Directory, Guard};
#[cfg(not(unix))]
mod unsupported;
#[cfg(not(unix))]
pub(crate) use unsupported::{Directory, Guard};

pub(crate) struct Storage {
    pub directory: Directory,
    pub repository_id: String,
    pub worktree_id: String,
}
impl Storage {
    pub fn open(root: &Path, create: bool) -> Result<Option<Self>> {
        let root = root
            .canonicalize()
            .map_err(|_| error("HOOK_PATH", "repository root is unavailable"))?;
        if root.to_str().is_none_or(|text| text.len() > 4096) {
            return Err(error("HOOK_PATH", "repository root must be bounded UTF-8"));
        }
        let common = git_path(
            &root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?;
        let parent = Directory::open(&common, false)?;
        let repo_id =
            crate::fingerprint(&serde_json::json!({"path":common,"identity":parent.identity()?}));
        let workspace = Directory::open(&root, false)?;
        let worktree_id =
            crate::fingerprint(&serde_json::json!({"path":root,"identity":workspace.identity()?}));
        let Some(directory) = parent.child("astral-hooks", create, true)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            directory,
            repository_id: repo_id,
            worktree_id,
        }))
    }
    pub fn read(&self, file: &str, limit: usize) -> Result<Option<Vec<u8>>> {
        Ok(self.directory.read(file, limit, true)?.map(|s| s.bytes))
    }
    pub fn write(&self, file: &str, bytes: &[u8], expected_sha: Option<&str>) -> Result<()> {
        self.directory.write(file, bytes, expected_sha, 0o600)
    }
    pub fn try_lock(&self) -> Result<Guard> {
        self.directory.lock()
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Notices {
    entries: Vec<(String, String)>,
}
/// True means emit. Callers emit on errors too: cache failure never hides advice.
pub(crate) fn notify_once(
    root: &Path,
    audience: &str,
    session: &str,
    digest: &str,
) -> Result<bool> {
    if audience.len() > 128
        || session.len() > 4096
        || digest.len() != 64
        || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(error("HOOK_CACHE_INPUT", "invalid notification metadata"));
    }
    let store = Storage::open(root, true)?
        .ok_or_else(|| error("HOOK_STORAGE", "private storage unavailable"))?;
    let _lock = store.try_lock()?;
    let previous = store.read("notices.json", 16_384)?;
    let mut notices: Notices = previous
        .as_ref()
        .map(|bytes| serde_json::from_slice(bytes))
        .transpose()
        .map_err(|_| error("HOOK_CACHE_INVALID", "notification cache is invalid"))?
        .unwrap_or_default();
    if notices.entries.len() > 64
        || notices.entries.iter().any(|(k, v)| {
            k.len() != 64
                || v.len() != 64
                || !k.bytes().chain(v.bytes()).all(|b| b.is_ascii_hexdigit())
        })
    {
        return Err(error(
            "HOOK_CACHE_INVALID",
            "notification cache exceeds its contract",
        ));
    }
    let key = crate::fingerprint(&serde_json::json!([
        store.repository_id,
        store.worktree_id,
        audience,
        session
    ]));
    if notices
        .entries
        .iter()
        .any(|(k, v)| k == &key && v == digest)
    {
        return Ok(false);
    }
    notices.entries.retain(|(k, _)| k != &key);
    if notices.entries.len() == 64 {
        notices.entries.remove(0);
    }
    notices.entries.push((key, digest.into()));
    let bytes = serde_json::to_vec(&notices)
        .map_err(|_| error("HOOK_CACHE_INVALID", "cannot encode notification cache"))?;
    store.write(
        "notices.json",
        &bytes,
        previous.as_ref().map(|b| crate::hash(b)).as_deref(),
    )?;
    Ok(true)
}

#[cfg(all(test, unix))]
mod tests;
