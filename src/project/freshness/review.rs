//! Explicit review acknowledgement; preview tokens bind the current checkout,
//! target manifest and observed inputs. They are not test or author signatures.
use super::*;
use std::path::Path;

pub fn review(root: &Path, subsystem: &str, apply: Option<&str>) -> Result<Value> {
    #[cfg(unix)]
    {
        implementation(root, subsystem, apply)
    }
    #[cfg(not(unix))]
    {
        let _ = (root, subsystem, apply);
        Err(error(
            "UNSUPPORTED_PLATFORM",
            "context review requires Unix",
        ))
    }
}

#[cfg(unix)]
fn implementation(root: &Path, subsystem: &str, apply: Option<&str>) -> Result<Value> {
    use crate::hooks::storage::{Directory, MAX_BYTES};
    let root = root
        .canonicalize()
        .map_err(|_| error("INVALID_ROOT", "repository root is unavailable"))?;
    let project = Project::load(&root)?;
    let reference = subsystem
        .starts_with("knowledge:")
        .then(|| super::super::knowledge::Reference::parse(subsystem))
        .transpose()?;
    let id = reference
        .as_ref()
        .map(|r| r.subsystem.as_str())
        .unwrap_or_else(|| subsystem.strip_prefix("subsystem:").unwrap_or(subsystem));
    let key = reference
        .as_ref()
        .map(|r| r.key())
        .unwrap_or_else(|| id.into());
    let observation = project.freshness.get(&key).ok_or_else(|| {
        error(
            "FRESHNESS_NOT_CONFIGURED",
            "choose a declared knowledge entry or a subsystem with explicit freshness inputs",
        )
    })?;
    let path = join(&project.subsystems[id].directory, "subsystem.toml")?;
    let fingerprint = observation.fingerprint.as_deref().ok_or_else(|| error("FRESHNESS_UNAVAILABLE", "cannot acknowledge unavailable inputs; inspect context freshness and repair the declared paths"))?;
    let plan_sha256 = crate::fingerprint(&json!({
        "schema_version": 1, "root": root, "subsystem": id, "reference": observation.reference(),
        "manifest": project.sources[&path], "fingerprint": fingerprint,
    }));
    let mut result = json!({
        "schema_version": 1, "operation": "context_review", "applied": false,
        "root": root, "subsystem": id, "manifest": path,
        "reference": observation.reference(),
        "plan_sha256": plan_sha256, "freshness": [observation],
        "scope": if reference.is_some() { "Acknowledges only this entry declaration, its document or marked region, and its explicit code files. Does not verify correctness, tests, core knowledge or other entries." } else { "Acknowledges review of the declared code files against current core and subsystem/dependency documents. Does not run tests, save a session, stage files or verify prose correctness." }
    });
    let Some(expected) = apply else {
        return project.output(result);
    };
    if expected != plan_sha256 {
        return Err(stale());
    }
    let storage_error = |e: super::super::Error| {
        if e.code == "HOOK_CHANGED" {
            stale()
        } else {
            error(
                "FRESHNESS_WRITE_FAILED",
                "review requires owned, confined, non-writable-by-others files and directories; another writer or a storage failure may prevent publication",
            )
        }
    };
    let mut directories = vec![Directory::open(&root, false).map_err(storage_error)?];
    for part in project.subsystems[id].directory.split('/') {
        let next = directories
            .last()
            .expect("root directory exists")
            .child(part, false, false)
            .map_err(storage_error)?
            .ok_or_else(stale)?;
        directories.push(next);
    }
    // All context reviewers and JSONL work writers cooperate on .astral's inode.
    let _lock = directories[1].lock_directory().map_err(storage_error)?;
    let parent = directories.last().expect("subsystem directory exists");
    let file = parent
        .read("subsystem.toml", MAX_BYTES, false)
        .map_err(storage_error)?
        .ok_or_else(stale)?;
    if file.sha256 != project.sources[&path].sha256 {
        return Err(stale());
    }
    let mut manifest: toml_edit::DocumentMut = std::str::from_utf8(&file.bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(stale)?;
    if let Some(reference) = &reference {
        update_entry(&mut manifest, &reference.entry, fingerprint)?;
    } else {
        manifest["freshness"]["reviewed_fingerprint"] = toml_edit::value(fingerprint);
    }
    let bytes = manifest.to_string().into_bytes();
    if bytes.len() > MAX_BYTES {
        return Err(error(
            "LIMIT_EXCEEDED",
            "reviewed subsystem manifest exceeds its file budget",
        ));
    }
    // Reobserve immediately before publishing. Ordinary editors do not take our
    // lock; this detects observed edits, not an atomic whole-workspace snapshot.
    let mut limits = super::super::Limits::default();
    limits.total_bytes = limits
        .total_bytes
        .saturating_sub(bytes.len().saturating_sub(file.bytes.len()));
    let current = Project::load_with_limits(&root, limits)?;
    if current.sources.get(&path).map(|s| s.sha256.as_str()) != Some(file.sha256.as_str())
        || current
            .freshness
            .get(&key)
            .and_then(|o| o.fingerprint.as_deref())
            != Some(fingerprint)
    {
        return Err(stale());
    }
    for directory in &directories {
        directory.unchanged().map_err(storage_error)?;
    }
    result["applied"] = json!(true);
    result["freshness"][0]["reviewed_fingerprint"] = json!(fingerprint);
    result["freshness"][0]["state"] = json!("unchanged");
    let result = project.output(result)?;
    parent
        .write("subsystem.toml", &bytes, Some(&file.sha256), file.mode)
        .map_err(storage_error)?;
    Ok(result)
}

#[cfg(unix)]
fn update_entry(manifest: &mut toml_edit::DocumentMut, id: &str, fingerprint: &str) -> Result<()> {
    let knowledge = manifest.get_mut("knowledge").ok_or_else(stale)?;
    if let Some(entries) = knowledge.as_array_of_tables_mut() {
        let entry = entries
            .iter_mut()
            .find(|e| e.get("id").and_then(toml_edit::Item::as_str) == Some(id))
            .ok_or_else(stale)?;
        entry["reviewed_fingerprint"] = toml_edit::value(fingerprint);
    } else if let Some(entries) = knowledge.as_array_mut() {
        let entry = entries
            .iter_mut()
            .filter_map(toml_edit::Value::as_inline_table_mut)
            .find(|e| e.get("id").and_then(toml_edit::Value::as_str) == Some(id))
            .ok_or_else(stale)?;
        entry.insert("reviewed_fingerprint", fingerprint.into());
    } else {
        return Err(stale());
    }
    Ok(())
}

#[cfg(unix)]
fn stale() -> super::super::Error {
    error(
        "FRESHNESS_REVIEW_STALE",
        "review inputs changed; inspect a new context review preview before applying",
    )
}
