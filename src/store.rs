use crate::engine::Lane;
use anyhow::Context;
use serde_json::Value;
use std::{
    collections::HashMap,
    fs::File,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::{
    io::AsyncWriteExt,
    sync::{Mutex as AsyncMutex, OwnedMutexGuard},
};

pub struct Store {
    root: PathBuf,
    lanes: Mutex<HashMap<String, Arc<AsyncMutex<Option<Lane>>>>>,
    ledger: AsyncMutex<tokio::fs::File>,
    capacity: usize,
    max_file_bytes: usize,
    _lock: File,
}

impl Store {
    pub async fn open(
        root: PathBuf,
        capacity: usize,
        max_file_bytes: usize,
    ) -> anyhow::Result<Self> {
        tokio::fs::create_dir_all(root.join("lanes")).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).await?;
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("process.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("another proxy owns this state directory")?;
        let mut lanes = HashMap::new();
        let mut entries = tokio::fs::read_dir(root.join("lanes")).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(id) = name.strip_suffix(".json").filter(|id| valid_id(id)) {
                anyhow::ensure!(
                    lanes.len() < capacity,
                    "state directory exceeds max-sessions"
                );
                lanes.insert(id.into(), Arc::new(AsyncMutex::new(None)));
            }
        }
        let ledger = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("ledger.jsonl"))
            .await?;
        Ok(Self {
            root,
            lanes: Mutex::new(lanes),
            ledger: AsyncMutex::new(ledger),
            capacity,
            max_file_bytes,
            _lock: lock,
        })
    }
    pub async fn lane(&self, id: &str) -> anyhow::Result<OwnedMutexGuard<Option<Lane>>> {
        anyhow::ensure!(valid_id(id), "invalid lane identifier");
        let lane = {
            let mut lanes = self
                .lanes
                .lock()
                .map_err(|_| anyhow::anyhow!("lane map poisoned"))?;
            if !lanes.contains_key(id) {
                anyhow::ensure!(
                    lanes.len() < self.capacity,
                    "max-sessions reached; use another state directory or increase the limit"
                );
                lanes.insert(id.into(), Arc::new(AsyncMutex::new(None)));
            }
            lanes[id].clone()
        };
        // Serialize the entire upstream lifecycle for a lane. Other lanes proceed independently.
        let mut guard = lane.lock_owned().await;
        if guard.is_none() {
            let path = self.root.join("lanes").join(format!("{id}.json"));
            let state = match tokio::fs::metadata(&path).await {
                Ok(meta) => {
                    anyhow::ensure!(
                        meta.len() <= self.max_file_bytes as u64,
                        "lane snapshot exceeds configured limit"
                    );
                    let bytes = tokio::fs::read(path).await?;
                    let state: Lane = serde_json::from_slice(&bytes)
                        .context("invalid lane snapshot; state was not silently reset")?;
                    anyhow::ensure!(state.version == 1, "unsupported lane snapshot version");
                    state
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Lane::default(),
                Err(e) => return Err(e.into()),
            };
            *guard = Some(state);
        }
        Ok(guard)
    }
    pub async fn commit(&self, id: &str, lane: &Lane) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(lane)?;
        anyhow::ensure!(
            bytes.len() <= self.max_file_bytes,
            "lane snapshot exceeds configured limit"
        );
        let dir = self.root.join("lanes");
        let temp = dir.join(format!("{id}.tmp"));
        let mut file = tokio::fs::File::create(&temp).await?;
        file.write_all(&bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(temp, dir.join(format!("{id}.json"))).await?;
        #[cfg(unix)]
        {
            tokio::task::spawn_blocking(move || File::open(dir)?.sync_all()).await??;
        }
        Ok(())
    }
    pub async fn record(&self, event: &Value) {
        let mut bytes = serde_json::to_vec(event).expect("JSON Value is serializable");
        bytes.push(b'\n');
        let mut ledger = self.ledger.lock().await;
        if let Err(e) = ledger.write_all(&bytes).await {
            eprintln!("ledger write failed: {e}");
        }
    }
}

fn valid_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|c| c.is_ascii_hexdigit())
}
