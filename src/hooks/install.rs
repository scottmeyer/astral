//! Explicit reviewed installation; foreign hooks/configuration are never replaced.
mod codex;
use super::storage::{self, Directory, MAX_BYTES, Snapshot, Storage, error};
use crate::project::Result;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub use super::GIT_EVENTS;
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Git,
    Codex,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Install,
    Uninstall,
}
#[derive(Debug, Clone, Serialize)]
pub struct FileState {
    pub sha256: Option<String>,
    pub identity: Option<String>,
    pub mode: Option<u32>,
    pub state: String,
}
impl FileState {
    fn of(value: Option<&Snapshot>) -> Self {
        Self {
            sha256: value.map(|v| v.sha256.clone()),
            identity: value.map(|v| v.identity.clone()),
            mode: value.map(|v| v.mode),
            state: if value.is_some() { "present" } else { "absent" }.into(),
        }
    }
}
#[derive(Debug, Serialize)]
pub struct Plan {
    pub schema_version: u32,
    pub target: Target,
    pub action: Action,
    pub root: PathBuf,
    pub common_git_dir: PathBuf,
    pub repository_id: String,
    pub worktree_id: String,
    pub executable: Option<PathBuf>,
    pub command: String,
    pub executable_lookup: &'static str,
    pub registration_sha256: Option<String>,
    pub files: BTreeMap<String, FileState>,
    pub blockers: Vec<String>,
    pub changes: Vec<String>,
    pub scope: &'static str,
    pub plan_sha256: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Registration {
    schema_version: u32,
    repository_id: String,
    executable: String,
    enabled: bool,
    entries: BTreeMap<String, String>,
}
struct Prepared {
    plan: Plan,
    registration: Option<Registration>,
    desired: Registration,
    codex_bytes: Option<Vec<u8>>,
    before: Option<Snapshot>,
}
fn encode<T: Serialize>(v: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(v).map_err(|_| error("HOOK_ENCODING", "cannot encode hook metadata"))
}
fn registration(
    store: Option<&Storage>,
    target: Target,
    worktree: &str,
) -> Result<(String, Option<Vec<u8>>, Option<Registration>)> {
    let file = match target {
        Target::Git => "git.json".into(),
        Target::Codex => format!("codex-{worktree}.json"),
    };
    let bytes = store.map(|s| s.read(&file, 16_384)).transpose()?.flatten();
    let parsed: Option<Registration> = bytes
        .as_ref()
        .map(|v| serde_json::from_slice(v))
        .transpose()
        .map_err(|_| error("HOOK_REGISTRATION_INVALID", "hook registration is invalid"))?;
    if parsed.as_ref().is_some_and(|r| {
        r.schema_version != 1
            || r.entries.len() > 6
            || r.repository_id.len() != 64
            || r.executable.len() > 4096
    }) {
        return Err(error(
            "HOOK_REGISTRATION_INVALID",
            "hook registration exceeds its contract",
        ));
    }
    if let Some(reg) = &parsed {
        let valid = match target {
            Target::Git => {
                Path::new(&reg.executable).is_absolute()
                    && !reg.executable.contains('\0')
                    && reg.entries.len() == GIT_EVENTS.len()
                    && GIT_EVENTS.iter().all(|event| {
                        reg.entries.get(*event) == Some(&crate::hash(&shim(&reg.executable, event)))
                    })
            }
            Target::Codex => {
                reg.executable == "astral"
                    && (reg.entries.is_empty() || reg.entries == codex::groups())
            }
        };
        if !valid {
            return Err(error(
                "HOOK_REGISTRATION_INVALID",
                "registration does not describe generated Astral hooks",
            ));
        }
    }
    Ok((file, bytes, parsed))
}
fn configured(root: &Path) -> Result<bool> {
    let bytes = crate::workspace::git_config_for_completion(root)?;
    Ok(bytes.split(|b| *b == 0).any(|row| {
        row.split(|b| *b == b'\n')
            .next()
            .is_some_and(|key| key.eq_ignore_ascii_case(b"core.hookspath"))
    }))
}
fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}
fn shim(executable: &str, event: &str) -> Vec<u8> {
    let exe = shell_quote(executable);
    format!("#!/bin/sh\n# Astral owned advisory hook v1; registration controls activation.\nif [ -x {exe} ]; then\n  {exe} hook git {event} \"$@\" || :\nelse\n  printf '%s\\n' 'Astral advisory unavailable: reviewed executable is missing.' >&2\nfi\nexit 0\n").into_bytes()
}
fn prepare(root: &Path, executable: &Path, target: Target, action: Action) -> Result<Prepared> {
    let root = root
        .canonicalize()
        .map_err(|_| error("HOOK_PATH", "repository root unavailable"))?;
    if root.to_str().is_none()
        || !executable.is_absolute()
        || executable
            .to_str()
            .is_none_or(|s| s.len() > 4096 || s.contains('\0'))
    {
        return Err(error(
            "HOOK_PATH",
            "installation requires bounded absolute UTF-8 paths",
        ));
    }
    let common = storage::git_path(
        &root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let common_dir = Directory::open(&common, false)?;
    let repository_id =
        crate::fingerprint(&json!({"path":common,"identity":common_dir.identity()?}));
    let worktree_id = crate::fingerprint(
        &json!({"path":root,"identity":Directory::open(&root,false)?.identity()?}),
    );
    let store = Storage::open(&root, false)?;
    let (_, registration_bytes, registration) = registration(store.as_ref(), target, &worktree_id)?;
    if registration
        .as_ref()
        .is_some_and(|r| r.repository_id != repository_id)
    {
        return Err(error(
            "HOOK_REGISTRATION_MISMATCH",
            "hook registration belongs to another repository",
        ));
    }
    let executable = if target == Target::Codex {
        PathBuf::from("astral")
    } else {
        executable.to_owned()
    };
    let mut plan = Plan {
        schema_version: 1,
        target,
        action,
        root: root.clone(),
        common_git_dir: common,
        repository_id: repository_id.clone(),
        worktree_id,
        executable: (target == Target::Git).then(|| executable.clone()),
        command: match target {
            Target::Git => format!(
                "{} hook git EVENT",
                shell_quote(&executable.to_string_lossy())
            ),
            Target::Codex => "astral hook codex".into(),
        },
        executable_lookup: if target == Target::Git {
            "absolute_reviewed_path"
        } else {
            "destination_PATH"
        },
        registration_sha256: registration_bytes.as_ref().map(|b| crate::hash(b)),
        files: BTreeMap::new(),
        blockers: Vec::new(),
        changes: Vec::new(),
        scope: match target {
            Target::Git => {
                "Default Git hooks are shared by this repository's linked worktrees. No core.hooksPath or foreign hook is replaced. Uninstall disables private registration and retains every shim."
            }
            Target::Codex => {
                "Only this checkout's .codex/hooks.json is changed. Other groups and prior bytes are retained; destination PATH supplies Astral. Runtime trust, global configuration and feature settings are unchanged."
            }
        },
        plan_sha256: String::new(),
    };
    let mut desired = Registration {
        schema_version: 1,
        repository_id,
        executable: executable.to_string_lossy().into_owned(),
        enabled: action == Action::Install,
        entries: BTreeMap::new(),
    };
    let mut before = None;
    let mut codex_bytes = None;
    match target {
        Target::Git => {
            if configured(&root)? && action == Action::Install {
                plan.blockers.push(
                    "CONFIGURED_HOOKS_PATH: compose manually with the existing manager".into(),
                );
            }
            let hooks = common_dir.child("hooks", false, false)?;
            for event in GIT_EVENTS {
                let expected = crate::hash(&shim(&desired.executable, event));
                desired.entries.insert(event.into(), expected.clone());
                let observed = hooks
                    .as_ref()
                    .map(|d| d.read(event, 16_384, false))
                    .transpose();
                let current = match observed {
                    Ok(v) => v.flatten(),
                    Err(_) => {
                        plan.files.insert(
                            event.into(),
                            FileState {
                                sha256: None,
                                identity: None,
                                mode: None,
                                state: "unsafe".into(),
                            },
                        );
                        if action == Action::Install {
                            plan.blockers.push(format!("FOREIGN_HOOK:{event}"));
                        }
                        continue;
                    }
                };
                plan.files
                    .insert(event.into(), FileState::of(current.as_ref()));
                if let Some(current) = current {
                    let owned = registration
                        .as_ref()
                        .and_then(|r| r.entries.get(event))
                        .is_some_and(|sha| sha == &current.sha256);
                    if action == Action::Install
                        && (!owned || current.sha256 != expected || current.mode & 0o100 == 0)
                    {
                        plan.blockers
                            .push(format!("FOREIGN_OR_CHANGED_HOOK:{event}"));
                    }
                } else if action == Action::Install {
                    plan.changes.push(format!("create default {event}"));
                }
            }
            if action == Action::Install {
                plan.changes
                    .push("enable complete owned Git registration".into());
            } else {
                plan.changes
                    .push("disable Git registration; retain hook files".into());
                desired = registration.clone().unwrap_or(desired);
                desired.enabled = false;
            }
        }
        Target::Codex => {
            let repo = Directory::open(&root, false)?;
            let dir = repo.child(".codex", false, false)?;
            before = dir
                .as_ref()
                .map(|d| d.read("hooks.json", MAX_BYTES, false))
                .transpose()?
                .flatten();
            plan.files
                .insert(".codex/hooks.json".into(), FileState::of(before.as_ref()));
            desired.entries = codex::groups();
            let transformed = codex::transform(
                before.as_ref().map(|b| b.bytes.as_slice()),
                registration.as_ref().map(|r| &r.entries),
                &desired.entries,
                action,
            )?;
            plan.blockers.extend(transformed.blockers);
            codex_bytes = transformed.bytes;
            if action == Action::Uninstall {
                desired.entries.clear();
            }
            if codex_bytes.as_deref() != before.as_ref().map(|s| s.bytes.as_slice()) {
                plan.changes
                    .push("back up exact prior bytes and update only Astral matcher groups".into());
            }
        }
    }
    plan.plan_sha256 = crate::hash(&encode(&plan)?);
    Ok(Prepared {
        plan,
        registration,
        desired,
        codex_bytes,
        before,
    })
}
pub fn plan(root: &Path, executable: &Path, target: Target, action: Action) -> Result<Plan> {
    Ok(prepare(root, executable, target, action)?.plan)
}

fn applied(
    root: &Path,
    target: Target,
    action: Action,
    changed: bool,
    enabled: bool,
    plan_sha256: &str,
) -> Value {
    match target {
        Target::Git => {
            json!({"operation":"hooks_apply","target":target,"action":action,"changed":changed,"enabled":enabled,"plan_sha256":plan_sha256})
        }
        Target::Codex => {
            let configuration = match status(root) {
                Ok(value) => value["codex"].clone(),
                Err(error) => {
                    json!({"observation_error":error.code,"runtime_trust":"not_observed"})
                }
            };
            json!({"operation":"hooks_apply","target":target,"action":action,"changed":changed,"registration_enabled":enabled,"plan_sha256":plan_sha256,"configuration":configuration,"removal_scope":"exact_owned_groups_only","modified_or_unowned_callbacks":"retained; execution is not observed"})
        }
    }
}

pub fn apply(
    root: &Path,
    executable: &Path,
    target: Target,
    action: Action,
    expected_hash: &str,
) -> Result<Value> {
    let initial = prepare(root, executable, target, action)?;
    if initial.plan.plan_sha256 != expected_hash {
        return Err(error(
            "HOOK_PLAN_CHANGED",
            "installation evidence changed; review a new plan",
        ));
    }
    if !initial.plan.blockers.is_empty() {
        return Err(error(
            "HOOK_INSTALL_BLOCKED",
            "installation requires manual composition or review of conflicting files",
        ));
    }
    if action == Action::Uninstall && initial.registration.is_none() {
        return Ok(applied(root, target, action, false, false, expected_hash));
    }
    let store = Storage::open(root, true)?
        .ok_or_else(|| error("HOOK_STORAGE", "hook storage unavailable"))?;
    let _lock = store.try_lock()?;
    let prepared = prepare(root, executable, target, action)?;
    if prepared.plan.plan_sha256 != expected_hash {
        return Err(error(
            "HOOK_PLAN_CHANGED",
            "installation evidence changed before ownership was acquired",
        ));
    }
    let complete = match target {
        Target::Git => prepared.plan.files.values().all(|f| f.state == "present"),
        Target::Codex => {
            prepared.codex_bytes.as_deref() == prepared.before.as_ref().map(|s| s.bytes.as_slice())
        }
    };
    if complete && prepared.registration.as_ref() == Some(&prepared.desired) {
        return Ok(applied(
            root,
            target,
            action,
            false,
            prepared.desired.enabled,
            expected_hash,
        ));
    }
    let (registration_file, old_bytes, _) =
        registration(Some(&store), target, &prepared.plan.worktree_id)?;
    let mut old_hash = old_bytes.as_ref().map(|b| crate::hash(b));
    let mut disabled = prepared.desired.clone();
    disabled.enabled = false;
    if action == Action::Install {
        store.write(&registration_file, &encode(&disabled)?, old_hash.as_deref())?;
        old_hash = Some(crate::hash(&encode(&disabled)?));
    }
    match target {
        Target::Git => {
            if action == Action::Install {
                let common = Directory::open(&prepared.plan.common_git_dir, false)?;
                let hooks = common
                    .child("hooks", true, false)?
                    .ok_or_else(|| error("HOOK_STORAGE", "hooks directory unavailable"))?;
                for event in GIT_EVENTS {
                    if prepared.plan.files[event].state == "absent" {
                        hooks.write(
                            event,
                            &shim(&prepared.desired.executable, event),
                            None,
                            0o755,
                        )?;
                    }
                }
                for event in GIT_EVENTS {
                    let current = hooks.read(event, 16_384, false)?.ok_or_else(|| {
                        error("HOOK_INCOMPLETE", "hook installation is incomplete")
                    })?;
                    if prepared.desired.entries.get(event) != Some(&current.sha256)
                        || current.mode & 0o100 == 0
                    {
                        return Err(error(
                            "HOOK_CHANGED",
                            "owned hook changed before activation",
                        ));
                    }
                }
            }
        }
        Target::Codex => {
            if let Some(bytes) = prepared.codex_bytes {
                if prepared.before.as_ref().map(|b| b.bytes.as_slice()) != Some(bytes.as_slice()) {
                    if let Some(before) = &prepared.before {
                        let backup = format!("codex-backup-{}.json", before.sha256);
                        match store.read(&backup, MAX_BYTES)? {
                            None => store.write(&backup, &before.bytes, None)?,
                            Some(bytes) if bytes == before.bytes => {}
                            Some(_) => {
                                return Err(error(
                                    "HOOK_BACKUP_CHANGED",
                                    "prior-byte backup changed; existing files retained",
                                ));
                            }
                        }
                    }
                    let dir = Directory::open(&prepared.plan.root, false)?
                        .child(".codex", true, false)?
                        .ok_or_else(|| error("HOOK_STORAGE", "Codex directory unavailable"))?;
                    dir.write(
                        "hooks.json",
                        &bytes,
                        prepared.before.as_ref().map(|b| b.sha256.as_str()),
                        prepared.before.as_ref().map_or(0o644, |b| b.mode),
                    )?;
                }
            }
        }
    }
    store.write(
        &registration_file,
        &encode(&prepared.desired)?,
        old_hash.as_deref(),
    )?;
    let mut outcome = applied(
        root,
        target,
        action,
        true,
        prepared.desired.enabled,
        expected_hash,
    );
    outcome["scope"] = json!(prepared.plan.scope);
    outcome["retained_prior_bytes"] = json!(true);
    Ok(outcome)
}

pub fn git_enabled(root: &Path, executable: &Path) -> Result<bool> {
    let Some(store) = Storage::open(root, false)? else {
        return Ok(false);
    };
    let (_, _, Some(reg)) = registration(Some(&store), Target::Git, &store.worktree_id)? else {
        return Ok(false);
    };
    if !reg.enabled {
        return Ok(false);
    };
    if reg.repository_id != store.repository_id
        || Path::new(&reg.executable) != executable
        || configured(root)?
    {
        return Ok(false);
    };
    let common = storage::git_path(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    let Some(hooks) = Directory::open(&common, false)?.child("hooks", false, false)? else {
        return Ok(false);
    };
    for event in GIT_EVENTS {
        let Some(current) = hooks.read(event, 16_384, false)? else {
            return Ok(false);
        };
        if current.sha256 != crate::hash(&shim(&reg.executable, event))
            || reg.entries.get(event) != Some(&current.sha256)
            || current.mode & 0o100 == 0
        {
            return Ok(false);
        };
    }
    Ok(true)
}
pub fn status(root: &Path) -> Result<Value> {
    let store = Storage::open(root, false)?;
    let worktree = store.as_ref().map_or("", |s| s.worktree_id.as_str());
    let (_, _, git) = registration(store.as_ref(), Target::Git, worktree)?;
    let (_, _, codex) = registration(store.as_ref(), Target::Codex, worktree)?;
    let git_valid = git
        .as_ref()
        .map(|r| git_enabled(root, Path::new(&r.executable)))
        .transpose()?
        .unwrap_or(false);
    let root = root
        .canonicalize()
        .map_err(|_| error("HOOK_PATH", "repository root unavailable"))?;
    let dir = Directory::open(&root, false)?.child(".codex", false, false)?;
    let before = dir
        .as_ref()
        .map(|d| d.read("hooks.json", MAX_BYTES, false))
        .transpose()?
        .flatten();
    let observed = codex::observe(before.as_ref().map(|s| s.bytes.as_slice()))?;
    let groups_intact = observed.values().all(|count| *count == 1);
    let ownership_intact = groups_intact
        && codex.as_ref().is_some_and(|r| {
            r.entries == codex::groups()
                && store
                    .as_ref()
                    .is_some_and(|s| r.repository_id == s.repository_id)
        });
    Ok(json!({
        "schema_version":1,
        "repository_id":store.as_ref().map(|s|&s.repository_id),
        "worktree_id":store.as_ref().map(|s|&s.worktree_id),
        "git":{"registered":git.is_some(),"enabled":git_valid},
        "codex":{"registered":codex.is_some(),"registration_enabled":codex.as_ref().is_some_and(|r|r.enabled),"configured":groups_intact,"owned_configuration_intact":ownership_intact,"generated_group_counts":observed,"command":"astral hook codex","executable_lookup":"destination_PATH","runtime_trust":"not_observed"}
    }))
}
