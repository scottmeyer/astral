//! Bounded Git observations for launch/capture; never applies source edits.
use crate::codex::error;
use crate::project::Result;
use std::path::Path;

pub async fn read(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let root = root.to_owned();
    let args: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
    tokio::task::spawn_blocking(move || crate::workspace::git_read(&root, &args))
        .await
        .map_err(|_| error("GIT_FAILED", "Git observation task failed"))?
}

/// A newly created worktree reads committed context. Only the validated work
/// file may be carried explicitly; arbitrary source edits stay in their checkout.
pub async fn require_committed_context(root: &Path, work_path: &str) -> Result<()> {
    let status = read(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--ignored=matching",
            "--",
            ".astral",
        ],
    )
    .await?;
    for entry in status.split(|b| *b == 0).filter(|entry| !entry.is_empty()) {
        if entry.len() < 4
            || &entry[3..] != work_path.as_bytes()
            || entry[..2].iter().any(|b| b"RCUD".contains(b))
        {
            return Err(error(
                "WORKSPACE_UNCOMMITTED_CONTEXT",
                "commit .astral context changes before creating a worker; only the work-record file can be carried automatically",
            ));
        }
    }
    let tracked = read(root, &["ls-files", "-v", "-z", "--", ".astral"]).await?;
    for entry in tracked.split(|b| *b == 0).filter(|entry| !entry.is_empty()) {
        if entry.len() < 3 || (entry[0] != b'H' && &entry[2..] != work_path.as_bytes()) {
            return Err(error(
                "WORKSPACE_UNCOMMITTED_CONTEXT",
                "clear assume-unchanged or skip-worktree flags on .astral context before creating a worker",
            ));
        }
    }
    Ok(())
}

pub async fn revision(root: &Path) -> Result<(String, bool)> {
    let value = read(root, &["rev-parse", "--verify", "HEAD"]).await?;
    let value = std::str::from_utf8(&value)
        .map_err(|_| error("GIT_FAILED", "invalid revision"))?
        .trim();
    if ![40, 64].contains(&value.len()) || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error("GIT_FAILED", "invalid revision"));
    }
    let dirty = !read(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .await?
    .is_empty();
    Ok((value.to_owned(), dirty))
}
