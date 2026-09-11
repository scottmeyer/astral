use super::{MAX_ENTRIES, error};
use crate::project::Result;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub(super) struct Entry {
    pub mode: String,
    pub oid: String,
}
pub(super) fn oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && value.bytes().any(|b| b != b'0')
}
fn insert(
    entries: &mut BTreeMap<String, Entry>,
    header: &[u8],
    path: &[u8],
    index: bool,
    intent: bool,
    exact_path: Option<&str>,
) -> Result<()> {
    let fields: Vec<_> = header.split(|b| *b == b' ').collect();
    if fields.len() != 3 {
        return Err(error("SNAPSHOT_FORMAT", "invalid Git entry header"));
    }
    let string = |b| {
        std::str::from_utf8(b)
            .map_err(|_| error("SNAPSHOT_FORMAT", "Git entry metadata requires UTF-8"))
    };
    let mode = string(fields[0])?;
    let object = string(if index { fields[1] } else { fields[2] })?;
    if index && fields[2] != b"0" {
        return Err(error(
            "SNAPSHOT_UNMERGED",
            "candidate index contains unresolved context entries",
        ));
    }
    if !index && fields[1] != b"blob" {
        return Err(error(
            "SNAPSHOT_UNSAFE_MODE",
            "context tree contains a non-blob entry",
        ));
    }
    if !matches!(mode, "100644" | "100755") {
        return Err(error(
            "SNAPSHOT_UNSAFE_MODE",
            "context snapshot contains a symlink, gitlink or unsupported mode",
        ));
    }
    if !oid(object) {
        return Err(error("SNAPSHOT_FORMAT", "invalid Git blob identifier"));
    }
    let path = string(path)?;
    if path.len() > 4096
        || !exact_path.map_or_else(
            || path == ".astral" || path.starts_with(".astral/"),
            |expected| path == expected,
        )
        || path.contains(['\\', ':', '\0'])
        || path
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        return Err(error(
            "SNAPSHOT_PATH",
            "invalid or escaping Git context path",
        ));
    }
    if intent {
        return Ok(());
    }
    if entries.len() >= MAX_ENTRIES {
        return Err(error(
            "SNAPSHOT_LIMIT",
            "snapshot entry count exceeds limit",
        ));
    }
    if entries
        .insert(
            path.into(),
            Entry {
                mode: mode.into(),
                oid: object.into(),
            },
        )
        .is_some()
    {
        return Err(error("SNAPSHOT_FORMAT", "duplicate Git context entry"));
    }
    Ok(())
}
fn header_path(line: &[u8]) -> Result<(&[u8], &[u8])> {
    let position = line
        .iter()
        .position(|b| *b == b'\t')
        .ok_or_else(|| error("SNAPSHOT_FORMAT", "missing Git entry separator"))?;
    Ok((&line[..position], &line[position + 1..]))
}
pub(super) fn tree(raw: &[u8]) -> Result<BTreeMap<String, Entry>> {
    tree_path(raw, None)
}
pub(super) fn tree_path(raw: &[u8], exact_path: Option<&str>) -> Result<BTreeMap<String, Entry>> {
    let mut entries = BTreeMap::new();
    if !raw.is_empty() && raw.last() != Some(&0) {
        return Err(error("SNAPSHOT_FORMAT", "truncated tree enumeration"));
    }
    for line in raw.split(|b| *b == 0).filter(|l| !l.is_empty()) {
        let (header, path) = header_path(line)?;
        insert(&mut entries, header, path, false, false, exact_path)?;
    }
    Ok(entries)
}
pub(super) fn index(raw: &[u8]) -> Result<BTreeMap<String, Entry>> {
    index_path(raw, None)
}
pub(super) fn index_path(
    mut raw: &[u8],
    exact_path: Option<&str>,
) -> Result<BTreeMap<String, Entry>> {
    let mut entries = BTreeMap::new();
    let mut count = 0;
    while !raw.is_empty() {
        count += 1;
        if count > MAX_ENTRIES {
            return Err(error(
                "SNAPSHOT_LIMIT",
                "snapshot index entry count exceeds limit",
            ));
        }
        let nul = raw
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| error("SNAPSHOT_FORMAT", "truncated index path"))?;
        let (header, path) = header_path(&raw[..nul])?;
        raw = &raw[nul + 1..];
        let mut flags = None;
        for prefix in [
            b"  ctime: ".as_slice(),
            b"  mtime: ",
            b"  dev: ",
            b"  uid: ",
            b"  size: ",
        ] {
            let end = raw
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| error("SNAPSHOT_FORMAT", "truncated index flags"))?;
            let line = &raw[..end];
            raw = &raw[end + 1..];
            if !line.starts_with(prefix) || line.len() > 256 || !line.is_ascii() {
                return Err(error(
                    "SNAPSHOT_FORMAT",
                    "unsupported Git index debug layout",
                ));
            }
            if prefix == b"  size: " {
                let value = line
                    .windows(8)
                    .position(|w| w == b"\tflags: ")
                    .map(|p| &line[p + 8..])
                    .ok_or_else(|| error("SNAPSHOT_FORMAT", "missing index entry flags"))?;
                flags = std::str::from_utf8(value)
                    .ok()
                    .and_then(|s| u32::from_str_radix(s, 16).ok());
            }
        }
        let flags = flags.ok_or_else(|| error("SNAPSHOT_FORMAT", "invalid index flags"))?;
        insert(
            &mut entries,
            header,
            path,
            true,
            flags & 0x2000_0000 != 0,
            exact_path,
        )?;
    }
    Ok(entries)
}
