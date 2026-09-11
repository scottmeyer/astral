//! Literal bounded Git plumbing; shares the existing process deadlines and cleanup.
use super::{MAX_PATHS, error};
use crate::project::{Project, Result};
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(super) struct Entry {
    pub mode: String,
    pub oid: String,
}
pub(super) type Tree = BTreeMap<String, Entry>;

pub(super) fn read(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    crate::workspace::git_read(root, &args.iter().map(Into::into).collect::<Vec<_>>())
}
pub(super) fn text(root: &Path, args: &[&str]) -> Result<String> {
    let bytes = read(root, args)?;
    let s = std::str::from_utf8(&bytes)
        .map_err(|_| error("FINISH_GIT_FORMAT", "Git output is not UTF-8"))?;
    if s.len() > 4096 || s.contains('\0') {
        return Err(error("FINISH_LIMIT", "Git scalar output exceeds limit"));
    }
    Ok(s.trim_end_matches('\n').into())
}
pub(super) fn path_text(path: &Path) -> Result<String> {
    path.to_str()
        .filter(|s| s.len() <= 4096)
        .map(Into::into)
        .ok_or_else(|| error("FINISH_PATH", "completion paths require bounded UTF-8"))
}
pub(super) fn head(root: &Path) -> Result<String> {
    text(root, &["rev-parse", "--verify", "HEAD^{commit}"])
}

pub(super) fn target(root: &Path, branch: &str) -> Result<()> {
    if branch.is_empty()
        || branch.len() > 256
        || branch.starts_with('-')
        || branch.chars().any(char::is_control)
    {
        return Err(error(
            "FINISH_TARGET",
            "target must be an explicit local branch name",
        ));
    }
    read(root, &["check-ref-format", &format!("refs/heads/{branch}")])?;
    if text(root, &["symbolic-ref", "--quiet", "HEAD"])? != format!("refs/heads/{branch}")
        || text(
            root,
            &["rev-parse", "--path-format=absolute", "--show-toplevel"],
        )? != path_text(root)?
    {
        return Err(error(
            "FINISH_TARGET",
            "invoke finish from the checkout already on the requested target branch",
        ));
    }
    Ok(())
}

fn safe_config(root: &Path) -> Result<()> {
    // Status may execute clean filters; check config before invoking it. Do not
    // bypass an operator's signature policy or accept branch merge-option injection.
    let bytes = crate::workspace::git_config_for_completion(root)?;
    for entry in bytes.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let mut fields = entry.splitn(2, |b| *b == b'\n');
        let key = fields.next().unwrap_or_default().to_ascii_lowercase();
        let value = fields.next().unwrap_or_default();
        if key.starts_with(b"filter.") {
            return Err(error(
                "FINISH_EXTERNAL_FILTER",
                "completion does not run repository filters; use ordinary Git",
            ));
        }
        if (key.starts_with(b"branch.") && key.ends_with(b".mergeoptions"))
            || key == b"merge.verifysignatures"
        {
            return Err(error(
                "FINISH_GIT_POLICY",
                "configured merge options or signature verification require ordinary Git integration",
            ));
        }
        if key == b"core.attributesfile"
            || key == b"core.eol"
            || (key == b"core.autocrlf" && !value.eq_ignore_ascii_case(b"false"))
            || (key == b"core.symlinks" && value.eq_ignore_ascii_case(b"false"))
        {
            return Err(error(
                "FINISH_CHECKOUT_CONVERSION",
                "configured checkout conversions require ordinary Git integration",
            ));
        }
    }
    Ok(())
}

pub(super) fn clean(root: &Path) -> Result<()> {
    safe_config(root)?;
    let git_dir = text(root, &["rev-parse", "--absolute-git-dir"])?;
    for name in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "BISECT_LOG",
        "index.lock",
    ] {
        match std::fs::symlink_metadata(Path::new(&git_dir).join(name)) {
            Ok(_) => {
                return Err(error(
                    "FINISH_GIT_OPERATION",
                    "finish requires no in-progress Git operation or index lock",
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(error(
                    "FINISH_GIT_OPERATION",
                    "cannot inspect current Git operation state",
                ));
            }
        }
    }
    let attributes = text(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "info/attributes",
        ],
    )?;
    match std::fs::symlink_metadata(attributes) {
        Ok(_) => {
            return Err(error(
                "FINISH_CHECKOUT_CONVERSION",
                "Git info attributes require ordinary Git integration",
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(error(
                "FINISH_CHECKOUT_CONVERSION",
                "cannot inspect Git attribute policy",
            ));
        }
    }
    let committed = tree(root, "HEAD")?;
    if committed.values().any(|entry| entry.mode == "160000") {
        return Err(error(
            "FINISH_SUBMODULE",
            "completion does not inspect or update submodule worktrees; use ordinary Git",
        ));
    }
    if committed
        .keys()
        .any(|p| p.rsplit('/').next() == Some(".gitattributes"))
    {
        return Err(error(
            "FINISH_CHECKOUT_CONVERSION",
            "tracked attributes require ordinary Git integration",
        ));
    }
    let flags = read(root, &["ls-files", "-v", "-z"])?;
    if flags
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .any(|e| e.first() != Some(&b'H'))
    {
        return Err(error(
            "FINISH_INDEX_FLAGS",
            "clear skip-worktree, assume-unchanged or unmerged index entries before finishing",
        ));
    }
    if !read(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?
    .is_empty()
    {
        return Err(error(
            "FINISH_DIRTY",
            "finish requires clean checkouts; preserve changes explicitly, without automatic staging or stashing",
        ));
    }
    Ok(())
}

/// Ignored build output is harmless unless it overlaps incoming tracked paths.
/// Treat an ignored directory as a whole prefix; do not inspect its private files.
pub(super) fn ignored_collisions(root: &Path, incoming: &Tree) -> Result<()> {
    let bytes = read(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
        ],
    )?;
    for entry in bytes.split(|b| *b == 0).filter(|e| e.starts_with(b"!! ")) {
        let path = std::str::from_utf8(&entry[3..]).map_err(|_| {
            error(
                "FINISH_PATH",
                "ignored paths require UTF-8 for collision review",
            )
        })?;
        let normalized = path.trim_end_matches('/');
        if normalized.rsplit('/').next() == Some(".gitattributes")
            || incoming.keys().any(|p| {
                p == normalized
                    || p.strip_prefix(normalized)
                        .is_some_and(|tail| tail.starts_with('/'))
                    || normalized
                        .strip_prefix(p.as_str())
                        .is_some_and(|tail| tail.starts_with('/'))
            })
        {
            return Err(error(
                "FINISH_IGNORED_COLLISION",
                "ignored target content overlaps incoming files or attribute policy; preserve it before integration",
            ));
        }
    }
    Ok(())
}

pub(super) fn unchanged(root: &Path, branch: &str, commit: &str) -> Result<()> {
    target(root, branch)?;
    clean(root)?;
    if head(root)? != commit {
        return Err(error(
            "FINISH_PLAN_CHANGED",
            "a compared branch advanced; review a new plan",
        ));
    }
    Ok(())
}

pub(super) fn tree(root: &Path, revision: &str) -> Result<Tree> {
    let bytes = read(root, &["ls-tree", "-r", "-z", "--full-tree", revision])?;
    let mut tree = Tree::new();
    for line in bytes.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        if tree.len() >= MAX_PATHS {
            return Err(error(
                "FINISH_LIMIT",
                "committed tree exceeds completion path limit",
            ));
        }
        let split = line
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| error("FINISH_GIT_FORMAT", "invalid tree entry"))?;
        let (header, suffix) = line.split_at(split);
        let fields: Vec<_> = header.split(|b| *b == b' ').collect();
        if fields.len() != 3 {
            return Err(error("FINISH_GIT_FORMAT", "invalid tree metadata"));
        }
        let path = std::str::from_utf8(&suffix[1..])
            .map_err(|_| error("FINISH_PATH", "completion paths require UTF-8"))?;
        if path.len() > 4096 {
            return Err(error("FINISH_LIMIT", "committed path exceeds limit"));
        }
        let mode = std::str::from_utf8(fields[0])
            .map_err(|_| error("FINISH_GIT_FORMAT", "invalid tree mode"))?;
        let oid = std::str::from_utf8(fields[2])
            .map_err(|_| error("FINISH_GIT_FORMAT", "invalid object ID"))?;
        tree.insert(
            path.into(),
            Entry {
                mode: mode.into(),
                oid: oid.into(),
            },
        );
    }
    Ok(tree)
}

/// Every file observed by the confined project loader, including unselected
/// native data, must match its committed blob exactly. No filters or text output.
pub(super) fn committed_sources(root: &Path, project: &Project, tree: &Tree) -> Result<()> {
    for source in project.observed_sources() {
        let entry = tree
            .get(&source.path)
            .filter(|entry| matches!(entry.mode.as_str(), "100644" | "100755"))
            .ok_or_else(|| {
                error(
                    "FINISH_UNCOMMITTED_SOURCE",
                    "a declared project source is not a committed regular file",
                )
            })?;
        let args = ["cat-file".into(), "blob".into(), entry.oid.clone().into()];
        let bytes = crate::workspace::git_read_with_limit(root, &args, source.bytes.max(1))?;
        if bytes.len() != source.bytes || crate::hash(&bytes) != source.sha256 {
            return Err(error(
                "FINISH_SOURCE_MISMATCH",
                "observed project source bytes differ from the committed object",
            ));
        }
    }
    Ok(())
}

pub(super) fn blob(root: &Path, revision: &str, path: &str) -> Result<Vec<u8>> {
    if !relative(path) {
        return Err(error("FINISH_PATH", "invalid committed project path"));
    }
    read(root, &["show", &format!("{revision}:{path}")])
}
pub(super) fn relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && path.len() <= 4096
        && path
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

pub(super) fn fast_forward(root: &Path, commit: &str) -> Result<()> {
    // Shared plumbing disables hooks, fsmonitor, submodule recursion and auto
    // maintenance. Rejected signature policies are never silently weakened.
    read(root, &["merge", "--ff-only", "--no-edit", "--no-stat", "--no-progress",
        "--no-autostash", "--no-overwrite-ignore", commit]).map(|_| ())
        .map_err(|_| error("FINISH_APPLY_UNCERTAIN", "Git integration did not return success; retain all artifacts and replan before retrying"))
}
