//! Immutable, bounded tool-output storage. Handles are unguessable bearer capabilities.
//! A result is replaced only after its artifact has been durably published.
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};

pub const PAGE_LIMIT: usize = 16_384;
const FILE_LIMIT: usize = 64 * 1024 * 1024;

pub struct Archive {
    root: PathBuf,
    secret: Vec<u8>,
    max_bytes: u64,
    used: Mutex<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifact {
    version: u8,
    scope: String,
    sha256: String,
    format: String,
    data: String,
}

pub fn valid_handle(handle: &str) -> bool {
    handle.len() == 64 && handle.bytes().all(|c| c.is_ascii_hexdigit())
}

fn private_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn publish(root: &Path, path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce).map_err(|e| anyhow::anyhow!("archive entropy unavailable: {e}"))?;
    let temp = root.join(format!("{}.tmp", crate::hash(&nonce)));
    let result = (|| {
        let mut file = private_file(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        #[cfg(unix)]
        File::open(root)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

impl Archive {
    /// The owning Store's process lock excludes other writers to this directory.
    pub fn open(root: &Path, max_bytes: u64) -> anyhow::Result<Self> {
        fs::create_dir_all(root)?;
        ensure!(
            !fs::symlink_metadata(root)?.file_type().is_symlink(),
            "archive directory is a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
        }
        let key_path = root.join("key");
        if !key_path.exists() {
            ensure!(
                !fs::read_dir(root)?.any(|entry| entry
                    .is_ok_and(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))),
                "archive key is missing; restore it before using existing artifacts"
            );
            let mut secret = [0u8; 32];
            getrandom::fill(&mut secret)
                .map_err(|e| anyhow::anyhow!("archive entropy unavailable: {e}"))?;
            publish(root, &key_path, &secret)?;
        }
        ensure!(
            fs::symlink_metadata(&key_path)?.is_file() && fs::metadata(&key_path)?.len() == 32,
            "invalid archive key"
        );
        let secret = fs::read(key_path)?;
        let mut used = 0u64;
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "json") {
                ensure!(entry.file_type()?.is_file(), "invalid archive entry");
                used = used
                    .checked_add(entry.metadata()?.len())
                    .context("archive size overflow")?;
            }
        }
        Ok(Self {
            root: root.into(),
            secret,
            max_bytes,
            used: Mutex::new(used),
        })
    }

    fn handle(&self, scope: &str, format: &str, digest: &str) -> String {
        let mut input = self.secret.clone();
        input.extend(serde_json::to_vec(&(scope, format, digest)).unwrap());
        crate::hash(&input)
    }

    pub fn save(&self, scope: &str, format: &str, data: &str) -> anyhow::Result<String> {
        ensure!(valid_handle(scope), "invalid archive scope");
        let digest = crate::hash(data.as_bytes());
        let handle = self.handle(scope, format, &digest);
        let path = self.root.join(format!("{handle}.json"));
        let mut used = self
            .used
            .lock()
            .map_err(|_| anyhow::anyhow!("archive lock poisoned"))?;
        if path.exists() {
            self.load(&handle)?;
            return Ok(handle);
        }
        let artifact = Artifact {
            version: 1,
            scope: scope.into(),
            sha256: digest,
            format: format.into(),
            data: data.into(),
        };
        let bytes = serde_json::to_vec(&artifact)?;
        ensure!(bytes.len() <= FILE_LIMIT, "artifact too large");
        ensure!(
            used.saturating_add(bytes.len() as u64) <= self.max_bytes,
            "archive capacity reached"
        );
        publish(&self.root, &path, &bytes)?;
        *used += bytes.len() as u64;
        Ok(handle)
    }

    fn load(&self, handle: &str) -> anyhow::Result<Artifact> {
        ensure!(valid_handle(handle), "invalid artifact handle");
        let path = self.root.join(format!("{handle}.json"));
        let metadata = fs::symlink_metadata(&path)?;
        ensure!(
            metadata.is_file() && metadata.len() <= FILE_LIMIT as u64,
            "invalid artifact file"
        );
        let artifact: Artifact = serde_json::from_slice(&fs::read(path)?)?;
        ensure!(
            artifact.version == 1
                && crate::hash(artifact.data.as_bytes()) == artifact.sha256
                && self.handle(&artifact.scope, &artifact.format, &artifact.sha256) == handle,
            "artifact integrity failure"
        );
        Ok(artifact)
    }

    pub fn recall(&self, handle: &str, offset: usize, limit: usize) -> anyhow::Result<Value> {
        ensure!(
            limit > 0 && limit <= PAGE_LIMIT,
            "limit must be 1..16384 bytes"
        );
        let artifact = self.load(handle)?;
        let bytes = artifact.data.as_bytes();
        ensure!(offset <= bytes.len(), "offset exceeds artifact length");
        let end = offset.saturating_add(limit).min(bytes.len());
        let (encoding, data) = match std::str::from_utf8(&bytes[offset..end]) {
            Ok(text) => ("utf8", text.to_owned()),
            Err(_) => {
                use std::fmt::Write as _;
                let mut hex = String::with_capacity((end - offset) * 2);
                for byte in &bytes[offset..end] {
                    write!(hex, "{byte:02x}").unwrap();
                }
                ("hex", hex)
            }
        };
        Ok(
            json!({"handle":handle,"sha256":artifact.sha256,"format":artifact.format,"offset":offset,"next_offset":end,"total_bytes":bytes.len(),"eof":end==bytes.len(),"encoding":encoding,"data":data}),
        )
    }

    pub fn search(&self, handle: &str, query: &str, offset: usize) -> anyhow::Result<Value> {
        ensure!(
            !query.is_empty() && query.len() <= 1024,
            "query must be 1..1024 bytes"
        );
        let artifact = self.load(handle)?;
        ensure!(
            offset <= artifact.data.len() && artifact.data.is_char_boundary(offset),
            "invalid search offset"
        );
        let mut hits = artifact.data[offset..].match_indices(query);
        let matches: Vec<_> = hits
            .by_ref()
            .take(20)
            .map(|(index, text)| json!({"offset":offset+index,"length":text.len()}))
            .collect();
        let next_offset = matches
            .last()
            .map(|m| m["offset"].as_u64().unwrap() + m["length"].as_u64().unwrap());
        Ok(
            json!({"handle":handle,"matches":matches,"next_offset":next_offset,"more":hits.next().is_some(),"total_bytes":artifact.data.len()}),
        )
    }
}
