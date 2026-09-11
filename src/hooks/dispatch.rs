//! Bounded advisory callbacks. Only the helper writes optional notification metadata.
mod io;
use super::{codex, install, storage};
use crate::lifecycle::{self, Scope};
use crate::project::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

const UNAVAILABLE: &str = "Astral lifecycle observation unavailable. Run astral lifecycle check to inspect current context; no worker state was changed.";

#[derive(Deserialize, Serialize)]
pub struct Advice {
    pub notice: Option<String>,
}

/// Internal subprocess entry point. Callbacks impose a deadline on this entire
/// operation, including repository discovery, registration and notification I/O.
pub fn check_callback(
    root: &Path,
    scope: Scope,
    audience: &str,
    session_scope: &str,
    claimed_session: Option<&str>,
    require_git_registration: bool,
    force_notice: bool,
) -> Result<Advice> {
    if require_git_registration {
        let executable = std::env::current_exe()
            .map_err(|_| storage::error("HOOK_EXECUTABLE", "current executable unavailable"))?;
        if !install::git_enabled(root, &executable)? {
            return Ok(Advice { notice: None });
        }
    }
    let report = lifecycle::check(root, scope, None)?;
    let notice = lifecycle::notice(&report, claimed_session);
    let digest = crate::fingerprint(&serde_json::json!([report.fingerprint, notice]));
    let emit = storage::notify_once(root, audience, session_scope, &digest).unwrap_or(true)
        || force_notice;
    Ok(Advice {
        notice: emit.then_some(notice),
    })
}

pub fn git_callback(root: &Path, event: &str, manual: bool) {
    if std::env::var_os("ASTRAL_HOOK_DEPTH").is_some() || !super::GIT_EVENTS.contains(&event) {
        return;
    }
    let scope = match event {
        "pre-commit" | "pre-merge-commit" => Scope::Index,
        "post-commit" => Scope::Head,
        _ => Scope::Worktree,
    };
    let notice = match io::observe(root, scope, "git-terminal", "", None, !manual, false) {
        Ok(advice) => advice.notice,
        Err(()) => Some(UNAVAILABLE.into()),
    };
    if let Some(notice) = notice {
        use std::io::Write;
        let _ = writeln!(std::io::stderr().lock(), "{notice}");
    }
}

pub fn codex_callback() {
    // Read promptly: Codex writes input before starting its own command timeout.
    let event = io::input(codex::MAX_INPUT_BYTES)
        .ok()
        .and_then(|bytes| codex::parse(&bytes).ok().flatten());
    let output = match event {
        Some(event) if std::env::var_os("ASTRAL_HOOK_DEPTH").is_none() => {
            let audience = match event.audience() {
                codex::Audience::Model => "codex-model",
                codex::Audience::Ui => "codex-ui",
            };
            let advice = io::observe(
                &event.cwd,
                Scope::Worktree,
                audience,
                &event.session_id,
                Some(&event.session_id),
                false,
                matches!(event.kind, codex::EventKind::SessionStart(_)),
            );
            let notice = match advice {
                Ok(advice) => advice.notice.unwrap_or_default(),
                Err(()) => UNAVAILABLE.into(),
            };
            codex::render(&event, &notice)
        }
        _ => serde_json::json!({}),
    };
    // Broken pipes are advisory failures too; never panic or return exit 2.
    use std::io::Write;
    let _ = writeln!(std::io::stdout().lock(), "{output}");
}
