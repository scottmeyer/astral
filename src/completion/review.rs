//! Metadata-only comparisons of committed paths, work records and native artifacts.
use super::{MAX_PATHS, error, git};
use crate::project::{Project, ProjectManifest, Result, SourceHandle};
use crate::work_records::{Conflict, MergeResult};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub path: String,
    pub category: &'static str,
    pub base_object: Option<String>,
    pub target_object: Option<String>,
    pub worker_object: Option<String>,
    pub both_changed: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct RecordReview {
    pub path: String,
    pub outcome: &'static str,
    pub conflicts: Vec<Conflict>,
    pub changed_ids: Vec<String>,
    pub base_sha256: String,
    pub target_sha256: String,
    pub worker_sha256: String,
    pub worker_status: String,
    pub target_status: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ContextReview {
    pub selector: String,
    pub explicitly_selected: bool,
    pub explicit_choice_required: bool,
    pub divergent_native_histories: bool,
    pub selected_digest: Option<String>,
    pub selected_native_sha256: Option<String>,
    pub selected_sources: Vec<SourceHandle>,
    pub target_native_sha256: Vec<String>,
    pub worker_native_sha256: Vec<String>,
    pub selection_error: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub path: String,
    pub object_id: String,
    pub mode: String,
    pub observed_in: &'static str,
    pub retained_in_result: bool,
}

pub(super) fn changes(
    base: &git::Tree,
    target: &git::Tree,
    worker: &git::Tree,
    work_path: &str,
) -> Result<Vec<Change>> {
    let paths: BTreeSet<_> = base
        .keys()
        .chain(target.keys())
        .chain(worker.keys())
        .collect();
    if paths.len() > MAX_PATHS {
        return Err(error("FINISH_LIMIT", "compared path union exceeds limit"));
    }
    Ok(paths
        .into_iter()
        .filter(|p| base.get(*p) != target.get(*p) || base.get(*p) != worker.get(*p))
        .map(|path| {
            let object = |tree: &git::Tree| tree.get(path).map(|e| format!("{}:{}", e.mode, e.oid));
            Change {
                path: path.clone(),
                category: if path == work_path {
                    "work_records"
                } else if path.starts_with(".astral/") && path.contains("/bundles/") {
                    "native_artifact"
                } else if path.starts_with(".astral/") {
                    "readable_context_or_manifest"
                } else {
                    "code_or_other"
                },
                base_object: object(base),
                target_object: object(target),
                worker_object: object(worker),
                both_changed: base.get(path) != target.get(path)
                    && base.get(path) != worker.get(path)
                    && target.get(path) != worker.get(path),
            }
        })
        .collect())
}

fn work_path(root: &Path, revision: &str) -> Result<String> {
    let bytes = git::blob(root, revision, ".astral/project.toml")?;
    let manifest: ProjectManifest = toml::from_str(
        std::str::from_utf8(&bytes)
            .map_err(|_| error("FINISH_MANIFEST", "committed project manifest is not UTF-8"))?,
    )
    .map_err(|_| {
        error(
            "FINISH_MANIFEST",
            "committed project manifest schema is invalid",
        )
    })?;
    if manifest.schema_version != 1 || !git::relative(&manifest.work_items) {
        return Err(error(
            "FINISH_MANIFEST",
            "unsupported committed work-register reference",
        ));
    }
    Ok(format!(".astral/{}", manifest.work_items))
}

pub(super) fn records(
    root: &Path,
    base: &str,
    target: &str,
    worker: &str,
    work: &str,
) -> Result<RecordReview> {
    let paths = [
        work_path(root, base)?,
        work_path(root, target)?,
        work_path(root, worker)?,
    ];
    let bytes = [
        git::blob(root, base, &paths[0])?,
        git::blob(root, target, &paths[1])?,
        git::blob(root, worker, &paths[2])?,
    ];
    let parsed: Vec<_> = bytes
        .iter()
        .map(|b| crate::work_records::parse(b).map_err(|e| error(e.code, e.message)))
        .collect::<Result<_>>()?;
    let record = parsed[2]
        .iter()
        .find(|r| r.item.id == work)
        .ok_or_else(|| {
            error(
                "WORK_NOT_FOUND",
                "worker's committed register lacks this work record",
            )
        })?;
    let worker_status = serde_json::to_value(&record.item.status)
        .expect("status serializes")
        .as_str()
        .expect("status string")
        .into();
    let indexed: Vec<BTreeMap<_, _>> = parsed
        .iter()
        .map(|rows| {
            rows.iter()
                .map(|r| (r.item.id.clone(), r.digest.clone()))
                .collect()
        })
        .collect();
    let changed_ids = indexed
        .iter()
        .flat_map(|rows| rows.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|id| {
            indexed[0].get(*id) != indexed[1].get(*id) || indexed[0].get(*id) != indexed[2].get(*id)
        })
        .cloned()
        .collect();
    let (outcome, conflicts) = if paths[0] != paths[1] || paths[0] != paths[2] {
        ("work_path_changed", Vec::new())
    } else {
        match crate::work_records::merge(&bytes[0], &bytes[1], &bytes[2]) {
            Ok(MergeResult::Merged { .. }) => ("merged", Vec::new()),
            Ok(MergeResult::Conflicts { conflicts }) => ("conflicts", conflicts),
            Err(_) => ("merged_graph_invalid", Vec::new()),
        }
    };
    Ok(RecordReview {
        path: paths[2].clone(),
        outcome,
        conflicts,
        changed_ids,
        base_sha256: crate::hash(&bytes[0]),
        target_sha256: crate::hash(&bytes[1]),
        worker_sha256: crate::hash(&bytes[2]),
        worker_status,
        target_status: parsed[1].iter().find(|r| r.item.id == work).map(|r| {
            serde_json::to_value(&r.item.status)
                .expect("status serializes")
                .as_str()
                .expect("status string")
                .to_owned()
        }),
    })
}

fn native(project: &Project) -> Result<(BTreeSet<String>, BTreeSet<String>)> {
    let list = project.list()?;
    let mut hashes = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for context in list["contexts"].as_array().expect("project list schema") {
        if context["kind"] != "projection" {
            continue;
        }
        for artifact in project
            .native_artifacts(context["selector"].as_str().expect("selector string"), None)?
        {
            hashes.insert(artifact.bundle.manifest_sha256);
            paths.insert(artifact.manifest.path);
            paths.insert(artifact.payload.path);
        }
    }
    Ok((hashes, paths))
}

#[allow(clippy::too_many_arguments)] // Explicit compared projects/trees plus reviewed choice.
pub(super) fn contexts(
    target: &Project,
    worker: &Project,
    target_tree: &git::Tree,
    worker_tree: &git::Tree,
    binding_selector: &str,
    choice: Option<&str>,
    integrated: bool,
) -> Result<(ContextReview, Vec<Artifact>)> {
    let (target_hashes, target_paths) = native(target)?;
    let (worker_hashes, worker_paths) = native(worker)?;
    let divergent =
        !target_hashes.is_empty() && !worker_hashes.is_empty() && target_hashes != worker_hashes;
    let selector = choice.unwrap_or(binding_selector);
    let result_project = if integrated { target } else { worker };
    let result_tree = if integrated { target_tree } else { worker_tree };
    let mut context = ContextReview {
        selector: selector.into(),
        explicitly_selected: choice.is_some(),
        explicit_choice_required: divergent && choice.is_none(),
        divergent_native_histories: divergent,
        selected_digest: None,
        selected_native_sha256: None,
        selected_sources: Vec::new(),
        target_native_sha256: target_hashes.into_iter().collect(),
        worker_native_sha256: worker_hashes.into_iter().collect(),
        selection_error: None,
    };
    match result_project.launch_context(selector, None) {
        Ok(selected) => {
            if selected.current_context.sources.iter().any(|s| {
                result_tree
                    .get(&s.path)
                    .is_none_or(|e| !e.mode.starts_with("100"))
            }) {
                context.selection_error = Some("UNCOMMITTED_CONTEXT_SOURCE".into());
            } else {
                context.selected_digest = Some(selected.current_context.selection_digest);
                context.selected_native_sha256 =
                    selected.native.map(|b| b.summary().manifest_sha256);
                context.selected_sources = selected.current_context.sources;
            }
        }
        Err(e) => context.selection_error = Some(e.code.into()),
    }
    let paths: BTreeSet<_> = target_paths
        .into_iter()
        .chain(worker_paths)
        .chain(
            target_tree
                .keys()
                .chain(worker_tree.keys())
                .filter(|p| p.starts_with(".astral/") && p.contains("/bundles/"))
                .cloned(),
        )
        .collect();
    if paths.len() > MAX_PATHS {
        return Err(error(
            "FINISH_LIMIT",
            "native artifact path inventory exceeds limit",
        ));
    }
    let mut artifacts = Vec::new();
    for path in paths {
        for (tree, origin) in [(target_tree, "target"), (worker_tree, "worker")] {
            if let Some(entry) = tree.get(&path) {
                if origin == "worker" && target_tree.get(&path) == Some(entry) {
                    continue;
                }
                artifacts.push(Artifact {
                    path: path.clone(),
                    object_id: entry.oid.clone(),
                    mode: entry.mode.clone(),
                    observed_in: origin,
                    retained_in_result: result_tree.get(&path) == Some(entry),
                });
            }
        }
    }
    Ok((context, artifacts))
}
