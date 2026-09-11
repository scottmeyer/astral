//! Captured Git context, identity checks and fixed-command subprocess access.
use super::index_file::{self, Stamp};
use super::{SnapshotKind, error};
use crate::project::Result;
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub(super) struct Context {
    pub root: PathBuf,
    pub common_dir: PathBuf,
    pub git_dir: PathBuf,
    object_bytes: usize,
    environment: Vec<(OsString, OsString)>,
    original_environment: Vec<(OsString, OsString)>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Evidence {
    root: (u64, u64),
    common: (u64, u64),
    git: (u64, u64),
    index: Option<(PathBuf, Option<Stamp>)>,
}

fn selected(key: &OsStr) -> bool {
    let key = key.to_string_lossy();
    key.starts_with("GIT_CONFIG_")
        || matches!(
            key.as_ref(),
            "GIT_DIR"
                | "GIT_COMMON_DIR"
                | "GIT_WORK_TREE"
                | "GIT_INDEX_FILE"
                | "GIT_OBJECT_DIRECTORY"
                | "GIT_ALTERNATE_OBJECT_DIRECTORIES"
                | "GIT_NAMESPACE"
        )
}
fn environment() -> Result<Vec<(OsString, OsString)>> {
    let mut values: Vec<_> = std::env::vars_os().filter(|(k, _)| selected(k)).collect();
    values.sort();
    let bytes: usize = values.iter().map(|(k, v)| k.len() + v.len()).sum();
    if values.len() > 256 || bytes > 65_536 {
        return Err(error(
            "SNAPSHOT_CONTEXT_LIMIT",
            "Git context environment exceeds limits",
        ));
    }
    Ok(values)
}
fn output(bytes: Vec<u8>) -> Result<String> {
    if bytes.len() > 4096 || bytes.contains(&0) {
        return Err(error(
            "SNAPSHOT_FORMAT",
            "Git scalar output exceeds its limit",
        ));
    }
    String::from_utf8(bytes)
        .map(|s| s.trim_end_matches('\n').to_owned())
        .map_err(|_| error("SNAPSHOT_FORMAT", "Git metadata requires UTF-8"))
}
fn absolute(value: String, root: &Path) -> Result<PathBuf> {
    if value.is_empty() {
        return Err(error(
            "SNAPSHOT_CONTEXT",
            "Git returned an empty context path",
        ));
    }
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}
fn canonical(value: String, root: &Path) -> Result<PathBuf> {
    absolute(value, root)?
        .canonicalize()
        .map_err(|_| error("SNAPSHOT_CONTEXT", "Git context path is unavailable"))
}

fn location_paths(read: impl Fn(&[&str]) -> Result<String>) -> Result<[String; 3]> {
    // rev-parse can emit these independent paths in one process. Unusual paths
    // containing newlines (or exceeding the combined scalar budget) retain the
    // separate reads, so batching never changes path interpretation.
    if let Ok(batch) = read(&[
        "rev-parse",
        "--path-format=absolute",
        "--show-toplevel",
        "--absolute-git-dir",
        "--git-common-dir",
    ]) {
        let lines: Vec<_> = batch.split('\n').collect();
        if let [root, git, common] = lines.as_slice() {
            return Ok([(*root).into(), (*git).into(), (*common).into()]);
        }
    }
    Ok([
        read(&["rev-parse", "--path-format=absolute", "--show-toplevel"])?,
        read(&["rev-parse", "--absolute-git-dir"])?,
        read(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?,
    ])
}

impl Context {
    pub fn discover(input: &Path) -> Result<Self> {
        let input = input
            .canonicalize()
            .map_err(|_| error("INVALID_ROOT", "snapshot checkout is unavailable"))?;
        let isolated = |args: &[&str]| -> Result<String> {
            output(crate::workspace::git_read(
                &input,
                &args.iter().map(Into::into).collect::<Vec<_>>(),
            )?)
        };
        let [root, git_dir, common_dir] = location_paths(isolated)?;
        let root = canonical(root, &input)?;
        let git_dir = canonical(git_dir, &input)?;
        let common_dir = canonical(common_dir, &input)?;
        let object_bytes = match isolated(&["rev-parse", "--show-object-format"])?.as_str() {
            "sha1" => 20,
            "sha256" => 32,
            _ => return Err(error("SNAPSHOT_FORMAT", "unsupported Git object format")),
        };
        let original_environment = environment()?;
        let mut captured = original_environment.clone();
        for (key, value) in &mut captured {
            if matches!(
                key.to_str(),
                Some(
                    "GIT_DIR"
                        | "GIT_COMMON_DIR"
                        | "GIT_WORK_TREE"
                        | "GIT_INDEX_FILE"
                        | "GIT_OBJECT_DIRECTORY"
                )
            ) && !Path::new(value).is_absolute()
            {
                *value = input.join(&*value).into_os_string();
            }
        }
        if input != root
            && captured
                .iter()
                .any(|(k, _)| k == "GIT_ALTERNATE_OBJECT_DIRECTORIES")
        {
            return Err(error(
                "SNAPSHOT_CONTEXT",
                "capture from the checkout root when alternate object directories are configured",
            ));
        }
        // These are snapshot policy, not imported executable authority. Disable
        // promisor fetching and replace-ref substitution even when the caller enables them.
        captured.extend([
            ("GIT_NO_LAZY_FETCH".into(), "1".into()),
            ("GIT_NO_REPLACE_OBJECTS".into(), "1".into()),
            ("GIT_LITERAL_PATHSPECS".into(), "1".into()),
        ]);
        let context = Self {
            root,
            common_dir,
            git_dir,
            object_bytes,
            environment: captured,
            original_environment,
        };
        context.verify_identity()?;
        Ok(context)
    }
    pub fn read(&self, args: &[&str], limit: usize) -> Result<Vec<u8>> {
        let (_, bytes) = crate::workspace::git_snapshot_read(
            &self.root,
            &args.iter().map(Into::into).collect::<Vec<_>>(),
            &self.environment,
            false,
            limit,
        )
        .map_err(|e| match e.code {
            "WORKSPACE_GIT_LIMIT" => error(
                "SNAPSHOT_LIMIT",
                "Git snapshot output exceeded its byte limit",
            ),
            "WORKSPACE_GIT_TIMEOUT" => error(
                "SNAPSHOT_TIMEOUT",
                "Git snapshot read exceeded its deadline",
            ),
            _ => error(
                "SNAPSHOT_GIT_READ",
                "fixed Git snapshot read failed; no working-file fallback was used",
            ),
        })?;
        Ok(bytes)
    }
    pub fn text(&self, args: &[&str]) -> Result<String> {
        output(self.read(args, 4096)?)
    }
    pub fn optional(&self, args: &[&str]) -> Result<Option<String>> {
        let (success, bytes) = crate::workspace::git_snapshot_read(
            &self.root,
            &args.iter().map(Into::into).collect::<Vec<_>>(),
            &self.environment,
            true,
            4096,
        )
        .map_err(|_| error("SNAPSHOT_GIT_READ", "optional Git metadata read failed"))?;
        if success {
            Ok(Some(output(bytes)?))
        } else if bytes.is_empty() {
            Ok(None)
        } else {
            Err(error("SNAPSHOT_FORMAT", "unexpected optional Git output"))
        }
    }
    pub fn verify_environment(&self) -> Result<()> {
        if environment()? != self.original_environment {
            return Err(error(
                "SNAPSHOT_CONTEXT_CHANGED",
                "Git context environment changed during observation",
            ));
        }
        Ok(())
    }
    pub fn verify_identity(&self) -> Result<()> {
        let paths = location_paths(|args| self.text(args))?;
        for (actual, expected) in
            paths
                .into_iter()
                .zip([&self.root, &self.git_dir, &self.common_dir])
        {
            if canonical(actual, &self.root)? != *expected {
                return Err(error(
                    "SNAPSHOT_CONTEXT_MISMATCH",
                    "Git-supplied context redirects to a different repository or worktree",
                ));
            }
        }
        Ok(())
    }
    pub fn evidence(&self, kind: SnapshotKind) -> Result<Evidence> {
        self.verify_identity()?;
        let index = if kind == SnapshotKind::Index {
            let index = absolute(
                self.text(&["rev-parse", "--path-format=absolute", "--git-path", "index"])?,
                &self.root,
            )?;
            let stamp = index_file::inspect(&index, self.object_bytes)?;
            Some((index, stamp))
        } else {
            None
        };
        Ok(Evidence {
            root: identity(&self.root)?,
            common: identity(&self.common_dir)?,
            git: identity(&self.git_dir)?,
            index,
        })
    }
}

#[cfg(unix)]
fn identity(path: &Path) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::metadata(path)
        .map_err(|_| error("SNAPSHOT_CONTEXT", "Git directory identity is unavailable"))?;
    if !m.is_dir() {
        return Err(error("SNAPSHOT_CONTEXT", "Git identity is not a directory"));
    }
    Ok((m.dev(), m.ino()))
}
#[cfg(not(unix))]
fn identity(_: &Path) -> Result<(u64, u64)> {
    Err(error(
        "SNAPSHOT_UNSUPPORTED_PLATFORM",
        "snapshot process confinement currently requires Unix",
    ))
}
