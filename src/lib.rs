pub mod config;
pub mod economics;
pub mod engine;
pub mod launcher;
pub mod native_binding;
mod native_transport;
pub mod policy;
pub mod project;
pub mod proxy;
pub mod server;
pub mod store;
pub mod usage;
pub mod working;

use serde_json::Value;
use sha2::{Digest, Sha256};

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// serde_json's default map is ordered; arrays and all item fields stay intact.
pub fn fingerprint(value: &Value) -> String {
    hash(&serde_json::to_vec(value).expect("JSON Value is serializable"))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
