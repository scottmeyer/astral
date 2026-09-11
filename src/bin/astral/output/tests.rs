use super::*;
use serde_json::json;

#[test]
fn context_validation_is_human_and_leads_to_list() {
    let output = render(&json!({"status":"valid","project_id":"fixture","subsystems":2,"projections":1,"work_items":3,"observed_files":12,"native_artifacts":0})).unwrap();
    assert!(output.starts_with("Context valid: fixture"));
    assert!(output.contains("2 subsystems, 1 projections, 3 work items"));
    assert!(output.contains("astral context list"));
    assert!(!output.contains("schema_version"));
}

#[test]
fn inspection_lists_sources_without_echoing_document_or_native_bodies() {
    let value = json!({"mode":"inspect","project_id":"fixture","selection":{"id":"web","kind":"subsystem","subsystems":["web"],"work_item":"AST-one"},"sources":[{"path":".astral/core/web/README.md","bytes":10}],"selection_digest":"abc","native_artifacts":[{"projection":"p","bundle":{"manifest_sha256":"bundle","encrypted_content":"OPAQUE_SENTINEL"}}],"metadata":{"subsystems":{"web":{"purpose":"DOCUMENT_SENTINEL"}}},"launch_request":{"requested_route":"proxy","non_interactive":true,"codex_args":["-c","sandbox_mode=\"read-only\""]}});
    let out = render(&value).unwrap();
    assert!(out.contains(".astral/core/web/README.md"));
    assert!(out.contains("explicit --proxy"));
    assert!(out.contains("Inspection only"));
    assert!(!out.contains("OPAQUE_SENTINEL"));
    assert!(!out.contains("DOCUMENT_SENTINEL"));
}

#[test]
fn untrusted_control_characters_cannot_inject_terminal_lines() {
    let out = render(&json!({"contexts":[{"selector":"subsystem:web\nFAKE\u{001b}[31m","kind":"subsystem"}],"project_id":"p\rBAD"})).unwrap();
    assert!(!out.contains('\r'));
    assert!(!out.contains('\u{001b}'));
    assert!(!out.contains("\nFAKE"));
    assert!(out.contains("\\nFAKE"));
    assert!(out.contains("non-displayable"));
}

#[test]
fn commands_quote_literal_metacharacters_and_never_execute_them() {
    let mut out = Output::default();
    out.command(
        "Next",
        &[
            "astral",
            "--root",
            "/tmp/a'b $(touch sentinel)",
            "hooks",
            "status",
        ],
    );
    assert_eq!(
        out.finish().unwrap(),
        "Next: astral --root '/tmp/a'\\''b $(touch sentinel)' hooks status"
    );
}

#[test]
fn work_records_show_exact_update_digest_and_acceptance() {
    let digest = "a".repeat(64);
    let value = json!({"item":{"id":"AST-a","title":"Repair","status":"open","acceptance":["Real execution"]},"digest":digest});
    let out = render(&value).unwrap();
    assert!(out.contains(&digest));
    assert!(out.contains("Real execution"));
    assert!(out.contains("astral status --work AST-a"));
    let list = render(&json!({"path":".astral/work/items.jsonl","records":[value]})).unwrap();
    assert!(list.contains("--expected DIGEST"));
}

#[test]
fn merge_is_a_preview_and_conflicts_are_named() {
    let clean = render(&json!({"outcome":"merged","jsonl":"{\"id\":\"PRIVATE_BODY\"}\n"})).unwrap();
    assert!(clean.contains("No file was written"));
    assert!(clean.contains("--json"));
    assert!(!clean.contains("PRIVATE_BODY"));
    let conflicted =
        render(&json!({"outcome":"conflicts","conflicts":[{"id":"AST-x","kind":"modify_delete"}]}))
            .unwrap();
    assert!(conflicted.contains("AST-x: modify_delete"));
}

#[test]
fn recovery_uses_offered_repair_and_retains_exact_hash() {
    let hash = "b".repeat(64);
    let value = json!({"work":"AST-x","classification":"creation_acknowledged","repair":"complete_creation","binding":{"observation":{"ownership":"available","plan":{"root":"/different/worker"},"issues":[{"code":"WORKSPACE_INCOMPLETE","message":"acknowledgement pending"}]}},"issues":[],"plan_sha256":hash,"guidance":"Record proven creation"});
    let out = render(&value).unwrap();
    assert!(out.contains(&format!("astral recover --work AST-x --apply {hash}")));
    assert!(out.contains("same project checkout"));
    assert!(!out.contains("--root /different/worker"));
}

#[test]
fn blocked_recovery_never_suggests_applying() {
    let out = render(&json!({"work":"AST-x","classification":"blocked","repair":null,"binding":{},"plan_sha256":"hash","guidance":"Wait for the owner"})).unwrap();
    assert!(out.contains("Wait for the owner"));
    assert!(!out.contains("--apply"));
}

#[test]
fn finish_retains_explicit_context_root_hash_and_blockers() {
    let value = json!({"operation":"finish_plan","work":"AST-x","target_root":"/tmp/repo space","target_branch":"main","worker_branch":"astral/AST-x","state":"fast_forward_ready","context":{"selector":"projection:p","explicitly_selected":true},"changes":[],"blockers":[],"guidance":["Review and commit current context first."],"plan_sha256":"reviewed"});
    let out = render(&value).unwrap();
    assert!(out.contains("astral --root '/tmp/repo space' finish --work AST-x --into main --context projection:p --apply reviewed"));
    assert!(out.contains("Review and commit current context first."));
    let mut blocked = value;
    blocked["state"] = json!("manual_integration_required");
    blocked["blockers"] = json!(["DIVERGED_GIT_HISTORY"]);
    let out = render(&blocked).unwrap();
    assert!(out.contains("DIVERGED_GIT_HISTORY"));
    assert!(!out.contains("--apply"));
}

#[test]
fn hooks_distinguish_configuration_from_trust_and_keep_apply_command() {
    let plan = render(&json!({"target":"codex","action":"install","root":"/tmp/repo","plan_sha256":"exact-hash","blockers":[],"changes":["add owned groups"],"files":{}})).unwrap();
    assert!(plan.contains("--apply exact-hash"));
    let status = render(&json!({"git":{"enabled":false,"registered":false},"codex":{"configured":true,"registered":true,"registration_enabled":true,"owned_configuration_intact":true}})).unwrap();
    assert!(status.contains("Codex: configured"));
    assert!(status.contains("runtime trust/loading is not observed"));
    let removed = render(&json!({"operation":"hooks_apply","target":"codex","action":"uninstall","changed":true,"registration_enabled":false})).unwrap();
    assert!(removed.contains("may still run"));
}

#[test]
fn paginated_inventory_keeps_next_page_and_unknown_outcomes() {
    let out = render(&json!({"operation":"recovery_inventory","total_bindings":40,"offset":0,"next_offset":32,"bindings":[{"work":"AST-x","in_current_register":true,"observation":{"observation":{"ownership":"unknown","issues":[{"code":"WORKSPACE_CHANGED","message":"Preserve evidence"}]}}}],"launches":{"state_root":"/private/receipts","total_observations":50,"offset":0,"next_offset":32,"launches":[]}})).unwrap();
    assert!(out.contains("AST-x: ownership unknown"));
    assert!(out.contains("astral recover --offset 32"));
    assert!(out.contains("astral recover --launch-state-root /private/receipts --offset 32"));
}

#[test]
fn initializer_and_save_do_not_claim_commit_or_execution() {
    let init = render(&json!({"operation":"initialize","mode":"inspect","prompt":"PROMPT_SENTINEL","launch_request":{"requested_route":"direct"}})).unwrap();
    assert!(!init.contains("PROMPT_SENTINEL"));
    assert!(init.contains("without --inspect"));
    let save = render(&json!({"operation":"save","work":"AST-x","worktree":"/repo","publication":{"projection":"handoff","bundle_manifest":{"sha256":"hash","path":"manifest.json"},"payload":{"path":"window.json"}}})).unwrap();
    assert!(save.contains("Saved projection handoff"));
    assert!(save.contains("deliberately commit"));
}

#[test]
fn large_lists_are_explicitly_abbreviated() {
    let out = render(&json!({"contexts":vec![json!({"selector":"subsystem:web","kind":"subsystem"}); MAX_ROWS+1],"project_id":"fixture"})).unwrap();
    assert!(out.contains("1 more entries; use --json"));
    assert_eq!(out.matches("(subsystem)").count(), MAX_ROWS);
}

#[test]
fn unknown_shapes_fail_without_echoing_sensitive_values() {
    let err = render(&json!({"secret":"NEVER_ECHO"})).unwrap_err();
    assert_eq!(err.code, "HUMAN_OUTPUT_UNSUPPORTED");
    assert!(!err.message.contains("NEVER_ECHO"));
    assert!(err.message.contains("may already have applied changes"));
    assert!(!err.message.contains("rerun"));
    let err = error(&Error {
        code: "BAD",
        message: "oops\u{001b}[31m\nspoof".into(),
    });
    assert!(!err.contains('\u{001b}'));
    assert!(!err.contains("\nspoof"));
}

#[test]
fn identifiers_stay_plain_and_total_output_is_bounded() {
    assert_eq!(render(&json!({"id":"AST-abcd"})).unwrap(), "AST-abcd");
    let mut out = Output::default();
    out.line("x".repeat(Limits::default().output_bytes));
    assert_eq!(out.finish().unwrap_err().code, "LIMIT_EXCEEDED");
}

#[test]
fn lifecycle_diagnostics_explain_drift_and_preserve_the_requested_root() {
    let out = render(&json!({"operation":"lifecycle_check","root":"/tmp/repo space","head":"commit","diagnostics":["UNSTAGED_CONTEXT_DIFFERENCE","UNCOMMITTED_CONTEXT_DIFFERENCE"]})).unwrap();
    assert!(out.contains("Working context differs from the index"));
    assert!(out.contains("stage intended context alongside its code"));
    assert!(out.contains("(UNSTAGED_CONTEXT_DIFFERENCE)"));
    assert!(out.contains("git -C '/tmp/repo space' diff -- .astral"));
    assert!(out.contains("git -C '/tmp/repo space' diff --cached -- .astral"));
}

#[test]
fn lifecycle_invalid_context_and_uncertain_worker_offer_readonly_next_steps() {
    let out = render(&json!({"operation":"lifecycle_check","root":"/repo","head":"commit","worker":{"work":"AST-x","ownership":"unknown"},"diagnostics":["INDEX_CONTEXT_INVALID","WORKER_RECOVERY_REVIEW_REQUIRED","WORKER_NATIVE_SELECTION_CHANGED"]})).unwrap();
    assert!(out.contains("Staged context is invalid or incomplete"));
    assert!(out.contains("Validate working context: astral --root /repo context validate"));
    assert!(out.contains("Inspect worker: astral --root /repo status --work AST-x"));
    assert!(out.contains("Preview recovery: astral --root /repo recover --work AST-x"));
    assert!(out.contains("Preserve both bundles"));
    assert!(!out.contains("--apply"));
}

#[test]
fn follow_up_commands_keep_absolute_or_space_containing_invocation_roots() {
    let contexts =
        json!({"contexts":[{"selector":"subsystem:web","kind":"subsystem"}],"project_id":"p"});
    let absolute = render_in(&contexts, Path::new("/repo")).unwrap();
    assert!(absolute.contains("astral --root /repo project subsystem:web --inspect"));
    let recovery = json!({"operation":"recovery_inventory","total_bindings":40,"offset":0,"next_offset":32,"bindings":[]});
    let spaced = render_in(&recovery, Path::new("/tmp/repo space")).unwrap();
    assert!(spaced.contains("astral --root '/tmp/repo space' recover --offset 32"));
    assert!(spaced.contains("astral --root '/tmp/repo space' recover --work ID"));
    let work = render_in(
        &json!({"path":".astral/work/items.jsonl","records":[]}),
        Path::new("/repo"),
    )
    .unwrap();
    assert!(work.contains("astral --root /repo work update ID --status STATUS --expected DIGEST"));
    let hints = error_in(
        &Error {
            code: "WORK_NOT_FOUND",
            message: "missing work".into(),
        },
        Path::new("/tmp/repo space"),
    );
    assert!(hints.contains("cd -- '/tmp/repo space'"));
}

#[test]
fn explicit_command_roots_and_raw_codex_arguments_are_not_rewritten() {
    let mut out = Output::in_root(Path::new("/invoking/repo"));
    out.command(
        "Next",
        &["astral", "--root", "/target/repo", "hooks", "status"],
    );
    let rendered = out.finish().unwrap();
    assert_eq!(rendered.matches("--root").count(), 1);
    assert!(rendered.contains("--root /target/repo"));
    assert!(!rendered.contains("/invoking/repo"));
    let value = json!({"operation":"initialize","launch_request":{"codex_args":["astral","literal prompt"]}});
    let preview = render_in(&value, Path::new("/invoking/repo")).unwrap();
    assert!(preview.contains("Explicit Codex arguments: astral 'literal prompt'"));
    assert!(!preview.contains("Explicit Codex arguments: astral --root"));
}

#[cfg(unix)]
#[test]
fn non_utf8_roots_never_produce_lossy_follow_up_commands() {
    use std::os::unix::ffi::OsStrExt;
    let root = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/repo-\xff"));
    let value = json!({"status":"valid","project_id":"p","subsystems":0,"projections":0,"work_items":0,"observed_files":0});
    let out = render_in(&value, root).unwrap();
    assert!(out.contains("retain the original --root argument"));
    assert!(!out.contains('\u{fffd}'));
    assert!(!out.contains("Next: astral context list"));
    let hint = error_in(
        &Error {
            code: "WORK_NOT_FOUND",
            message: "missing work".into(),
        },
        root,
    );
    assert!(hint.contains("non-UTF-8 path cannot be shown"));
    assert!(!hint.contains('\u{fffd}'));
}
