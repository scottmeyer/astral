//! Preflight the index format before Git reads it. Git can freshen a split
//! index's shared-file timestamp during an otherwise read-only command.
use super::error;
use crate::project::Result;
use std::path::Path;
pub(super) const MAX_INDEX_BYTES: usize = 33_554_432;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Stamp {
    identity: (u64, u64),
    bytes: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    sha256: String,
}
fn invalid() -> crate::project::Error {
    error(
        "SNAPSHOT_INDEX_FORMAT",
        "candidate index format is malformed or unsupported",
    )
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(at..at + 4)
            .ok_or_else(invalid)?
            .try_into()
            .expect("four bytes"),
    ))
}
fn inspect_bytes(bytes: &[u8], hash_bytes: usize) -> Result<()> {
    if bytes.get(..4) != Some(b"DIRC") || bytes.len() < 12 + hash_bytes {
        return Err(invalid());
    }
    let version = u32_at(bytes, 4)?;
    if !matches!(version, 2..=4) {
        return Err(invalid());
    }
    let count = u32_at(bytes, 8)? as usize;
    let end = bytes.len() - hash_bytes;
    let mut cursor = 12usize;
    for _ in 0..count {
        let start = cursor;
        let flag_at = cursor.checked_add(40 + hash_bytes).ok_or_else(invalid)?;
        let flags = u16::from_be_bytes(
            bytes
                .get(flag_at..flag_at + 2)
                .ok_or_else(invalid)?
                .try_into()
                .expect("two bytes"),
        );
        cursor = flag_at + 2;
        if flags & 0x4000 != 0 {
            if version == 2 {
                return Err(invalid());
            }
            cursor = cursor.checked_add(2).ok_or_else(invalid)?;
        }
        if version == 4 {
            let mut width = 0;
            loop {
                let b = *bytes.get(cursor).ok_or_else(invalid)?;
                cursor += 1;
                width += 1;
                if width > 10 {
                    return Err(invalid());
                }
                if b & 0x80 == 0 {
                    break;
                }
            }
        }
        let nul = bytes
            .get(cursor..end)
            .ok_or_else(invalid)?
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(invalid)?;
        cursor = cursor.checked_add(nul + 1).ok_or_else(invalid)?;
        if version < 4 {
            let padding = (8 - (cursor - start) % 8) % 8;
            if bytes
                .get(cursor..cursor + padding)
                .ok_or_else(invalid)?
                .iter()
                .any(|b| *b != 0)
            {
                return Err(invalid());
            }
            cursor += padding;
        }
        if cursor > end {
            return Err(invalid());
        }
    }
    while cursor < end {
        let signature = bytes.get(cursor..cursor + 4).ok_or_else(invalid)?;
        let size = u32_at(bytes, cursor + 4)? as usize;
        cursor = cursor
            .checked_add(8)
            .and_then(|p| p.checked_add(size))
            .ok_or_else(invalid)?;
        if cursor > end {
            return Err(invalid());
        }
        if signature == b"link" {
            return Err(error(
                "SNAPSHOT_SPLIT_INDEX",
                "split indexes require unsupported timestamp-mutating Git reads; use an ordinary index for advisory checks",
            ));
        }
        if !signature[0].is_ascii_uppercase() {
            return Err(error(
                "SNAPSHOT_INDEX_EXTENSION",
                "candidate uses an unsupported mandatory index extension",
            ));
        }
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn inspect(path: &Path, hash_bytes: usize) -> Result<Option<Stamp>> {
    use std::{
        fs::OpenOptions,
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(error(
                "SNAPSHOT_INDEX",
                "candidate index is unavailable or linked",
            ));
        }
    };
    let meta = file
        .metadata()
        .map_err(|_| error("SNAPSHOT_INDEX", "index metadata is unavailable"))?;
    if !meta.is_file() {
        return Err(error("SNAPSHOT_INDEX", "candidate index must be regular"));
    }
    if meta.len() > MAX_INDEX_BYTES as u64 {
        return Err(error(
            "SNAPSHOT_LIMIT",
            "candidate index exceeds32MiB preflight budget",
        ));
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take(MAX_INDEX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error("SNAPSHOT_INDEX", "cannot read candidate index"))?;
    if bytes.len() > MAX_INDEX_BYTES {
        return Err(error(
            "SNAPSHOT_LIMIT",
            "candidate index exceeds32MiB preflight budget",
        ));
    }
    let stamp = |m: &std::fs::Metadata| Stamp {
        identity: (m.dev(), m.ino()),
        bytes: m.len(),
        modified: (m.mtime(), m.mtime_nsec()),
        changed: (m.ctime(), m.ctime_nsec()),
        sha256: crate::hash(&bytes),
    };
    let before = stamp(&meta);
    let after = stamp(
        &file
            .metadata()
            .map_err(|_| error("SNAPSHOT_INDEX", "index metadata is unavailable"))?,
    );
    if before != after || meta.len() != bytes.len() as u64 {
        return Err(error(
            "SNAPSHOT_CHANGED",
            "candidate index changed during preflight",
        ));
    }
    inspect_bytes(&bytes, hash_bytes)?;
    Ok(Some(after))
}
#[cfg(not(unix))]
pub(super) fn inspect(_: &Path, _: usize) -> Result<Option<Stamp>> {
    Err(error(
        "SNAPSHOT_UNSUPPORTED_PLATFORM",
        "index preflight requires Unix",
    ))
}
