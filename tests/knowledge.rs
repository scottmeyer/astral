#![cfg(unix)]
#[path = "support/knowledge_fixture.rs"]
mod fixture;
use astral::{
    lifecycle::{self, Scope},
    project::{Limits, Project, freshness, knowledge::Reference},
};
use fixture::{DOCUMENT, Fixture, MANIFEST, REGIONS};
use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn stable_references_recall_only_the_requested_region_and_preserve_review_independence() {
    let f = Fixture::new();
    assert_eq!(
        Reference::parse("knowledge:web/sessions")
            .unwrap()
            .to_string(),
        "knowledge:web/sessions"
    );
    for reference in [
        "web/sessions",
        "knowledge:web",
        "knowledge:web/../x",
        "knowledge:web/x/y",
        "knowledge:/x",
    ] {
        assert!(Reference::parse(reference).is_err());
    }
    let shown = f
        .project()
        .show_knowledge("knowledge:web/sessions")
        .unwrap();
    assert_eq!(shown["text"], "Session claim.\n");
    assert_eq!(
        shown["freshness"][0]["document"]["bytes"],
        "Session claim.\n".len()
    );
    assert_eq!(
        f.project().list_knowledge("projection:seed").unwrap()["knowledge"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    f.review("knowledge:web/sessions");
    assert_eq!(f.observation("routing")["state"], "unreviewed");
    f.review("knowledge:web/routing");
    let before = f.project().fresh_context("web", None).unwrap();
    assert_eq!(before.freshness.len(), 2);
    assert!(
        !serde_json::to_string(&before)
            .unwrap()
            .contains("SECRET_SESSION_CODE")
    );
    assert!(
        !serde_json::to_string(&before)
            .unwrap()
            .contains("SECRET_ROUTER_CODE")
    );
    f.put("src/session.rs", "changed session behavior");
    assert_eq!(f.observation("sessions")["state"], "needs_review");
    assert_eq!(f.observation("routing")["state"], "unchanged");
    assert_ne!(
        before.selection_digest,
        f.project()
            .fresh_context("web", None)
            .unwrap()
            .selection_digest
    );
    f.review("knowledge:web/sessions");
    f.put(
        DOCUMENT,
        REGIONS.replace("Routing claim.", "Updated routing claim."),
    );
    assert_eq!(f.observation("sessions")["state"], "unchanged");
    assert_eq!(f.observation("routing")["state"], "needs_review");
    f.put(".astral/core/TEST.md", "Changed shared instructions");
    assert_eq!(f.observation("sessions")["state"], "unchanged");
}

#[test]
fn invalid_markers_fail_locally_without_guessing_or_invalidating_other_entries() {
    for body in [
        REGIONS.replace("<!-- astral:end sessions -->", ""),
        REGIONS.replace(
            "<!-- astral:begin sessions -->",
            "<!-- astral:begin sessions -->\n<!-- astral:begin sessions -->",
        ),
        REGIONS.replace(
            "<!-- astral:begin sessions -->",
            "<!-- astral:end sessions -->",
        ),
        REGIONS.replace("Session claim.\n", ""),
    ] {
        let f = Fixture::new();
        f.review("knowledge:web/routing");
        f.put(DOCUMENT, &body);
        assert_eq!(f.observation("sessions")["state"], "unavailable");
        assert_eq!(f.observation("routing")["state"], "unchanged");
        assert!(
            f.project()
                .show_knowledge("knowledge:web/sessions")
                .is_err()
        );
        assert_eq!(
            freshness::review(f.root(), "knowledge:web/sessions", None)
                .unwrap_err()
                .code,
            "FRESHNESS_UNAVAILABLE"
        );
    }
}

#[test]
fn crlf_whole_document_and_missing_code_have_explicit_recall_behavior() {
    let f = Fixture::new();
    f.put(DOCUMENT, REGIONS.replace('\n', "\r\n"));
    assert_eq!(
        f.project()
            .show_knowledge("knowledge:web/sessions")
            .unwrap()["text"],
        "Session claim.\r\n"
    );
    f.define(&Fixture::entry("whole", None, "src/missing.rs"));
    assert_eq!(f.observation("whole")["state"], "unavailable");
    assert_eq!(
        f.project().show_knowledge("knowledge:web/whole").unwrap()["text"],
        REGIONS.replace('\n', "\r\n")
    );
    f.put(DOCUMENT, [0xff, 0xfe]);
    assert_eq!(
        f.observation("whole")["document"]["error_code"],
        "KNOWLEDGE_UTF8"
    );
}

#[test]
fn moving_the_document_retains_identity_but_requires_review() {
    let f = Fixture::new();
    f.review("knowledge:web/sessions");
    f.put(".astral/core/web/moved.md", REGIONS);
    let manifest = fs::read_to_string(f.root().join(MANIFEST)).unwrap();
    f.put(MANIFEST, manifest.replace("README.md", "moved.md"));
    assert_eq!(f.observation("sessions")["state"], "needs_review");
    let shown = f
        .project()
        .show_knowledge("knowledge:web/sessions")
        .unwrap();
    assert_eq!(shown["reference"], "knowledge:web/sessions");
    assert_eq!(shown["text"], "Session claim.\n");
}

#[test]
fn entry_review_is_narrow_and_stale_previews_cannot_overwrite_other_edits() {
    let f = Fixture::new();
    let preview = freshness::review(f.root(), "knowledge:web/sessions", None).unwrap();
    let before = fs::read(f.root().join(MANIFEST)).unwrap();
    f.put("src/session.rs", "changed after review");
    assert_eq!(
        freshness::review(
            f.root(),
            "knowledge:web/sessions",
            preview["plan_sha256"].as_str()
        )
        .unwrap_err()
        .code,
        "FRESHNESS_REVIEW_STALE"
    );
    assert_eq!(fs::read(f.root().join(MANIFEST)).unwrap(), before);
    let preview = freshness::review(f.root(), "knowledge:web/sessions", None).unwrap();
    f.review("knowledge:web/routing");
    assert_eq!(
        freshness::review(
            f.root(),
            "knowledge:web/sessions",
            preview["plan_sha256"].as_str()
        )
        .unwrap_err()
        .code,
        "FRESHNESS_REVIEW_STALE"
    );
    assert_eq!(f.observation("sessions")["state"], "unreviewed");
    assert_eq!(f.observation("routing")["state"], "unchanged");
    f.review("knowledge:web/sessions");
    assert!(
        fs::read_to_string(f.root().join(MANIFEST))
            .unwrap()
            .starts_with("# Kept comment\nschema_version=1")
    );
}

#[test]
fn inline_arrays_and_subsystem_baselines_use_the_same_review_contract() {
    let f = Fixture::new();
    f.define("knowledge=[{id='sessions',kind='rule',title='Sessions',document='README.md',region='sessions',inputs=['src/session.rs']}]\n[freshness]\ninputs=['src/router.rs']\n");
    f.review("web");
    f.review("knowledge:web/sessions");
    let observations = f.project().inspect_freshness("web").unwrap();
    assert!(
        observations["freshness"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["state"] == "unchanged")
    );
    f.put("src/session.rs", "knowledge-only declared code changed");
    let observations = f.project().inspect_freshness("web").unwrap();
    assert!(
        observations["freshness"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["state"] == "needs_review")
    );
    f.review("web");
    assert_eq!(f.observation("sessions")["state"], "needs_review");
}

#[test]
fn declarations_are_bounded_confined_unique_and_explicit() {
    let f = Fixture::new();
    let entry = Fixture::entry("sessions", Some("sessions"), "src/session.rs");
    for invalid in [
        format!("{entry}{entry}"),
        entry.replace("README.md", "../../outside.md"),
        entry.replace("src/session.rs", ".git/config"),
        entry.replace("kind='decision'", "kind='executable'"),
        entry.replace("title='sessions decision'", "title=''"),
        entry.replace("region='sessions'", "region='../x'"),
    ] {
        f.define(&invalid);
        assert!(Project::load(f.root()).is_err());
    }
    let entries = (0..257)
        .map(|i| Fixture::entry(&format!("entry-{i}"), Some("sessions"), "src/session.rs"))
        .collect::<String>();
    f.define(&entries);
    assert_eq!(
        Project::load(f.root()).err().unwrap().code,
        "KNOWLEDGE_LIMIT"
    );
    let f = Fixture::new();
    let project = Project::load_with_limits(
        f.root(),
        Limits {
            output_bytes: 100,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        project
            .show_knowledge("knowledge:web/sessions")
            .unwrap_err()
            .code,
        "LIMIT_EXCEEDED"
    );
    let entries = (0..65)
        .map(|i| {
            Fixture::entry(
                &format!("entry-{i}"),
                Some("sessions"),
                &format!("src/input-{i}.rs"),
            )
        })
        .collect::<String>();
    f.define(&entries);
    assert_eq!(
        Project::load(f.root()).err().unwrap().code,
        "FRESHNESS_INPUT_LIMIT"
    );
}

#[test]
fn git_views_keep_region_and_input_evidence_separate_and_hooks_do_not_quote_claims() {
    let f = Fixture::new();
    f.review("knowledge:web/sessions");
    f.review("knowledge:web/routing");
    f.git(&["init", "--initial-branch=main"]);
    f.git(&["add", "."]);
    f.git(&["commit", "-m", "baseline"]);
    f.put(
        DOCUMENT,
        REGIONS.replace("Session claim.", "Staged session claim."),
    );
    f.git(&["add", DOCUMENT]);
    f.put(
        DOCUMENT,
        REGIONS.replace("Session claim.", "Working session claim."),
    );
    f.review("knowledge:web/sessions");
    f.git(&["add", MANIFEST]);
    let report = lifecycle::check(f.root(), Scope::Index, None).unwrap();
    let state = |context: &lifecycle::Context, id: &str| {
        context
            .freshness
            .iter()
            .find(|o| o.knowledge.as_deref() == Some(id))
            .unwrap()
            .state
            .clone()
    };
    assert_eq!(
        state(&report.committed, "sessions"),
        freshness::State::Unchanged
    );
    assert_eq!(
        state(&report.index, "sessions"),
        freshness::State::NeedsReview
    );
    assert_eq!(
        state(&report.worktree, "sessions"),
        freshness::State::Unchanged
    );
    assert_eq!(state(&report.index, "routing"), freshness::State::Unchanged);
    assert!(!lifecycle::notice(&report, None).contains("session claim"));
    f.git(&["add", DOCUMENT]);
    assert_eq!(
        state(
            &lifecycle::check(f.root(), Scope::Index, None)
                .unwrap()
                .index,
            "sessions"
        ),
        freshness::State::Unchanged
    );
}

#[test]
fn cli_lists_recalls_reviews_and_reports_the_target_reference() {
    let f = Fixture::new();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_astral"))
            .current_dir(f.root())
            .args(args)
            .output()
            .unwrap()
    };
    let listed = run(&["context", "knowledge", "web"]);
    assert!(listed.status.success());
    assert!(String::from_utf8_lossy(&listed.stdout).contains("knowledge:web/sessions"));
    let shown = run(&["context", "show", "knowledge:web/sessions", "--json"]);
    let shown: Value = serde_json::from_slice(&shown.stdout).unwrap();
    assert_eq!(shown["text"], "Session claim.\n");
    let preview = run(&["context", "review", "knowledge:web/sessions", "--dry-run"]);
    assert!(
        String::from_utf8_lossy(&preview.stdout).contains("review knowledge:web/sessions --apply")
    );
    let applied = run(&[
        "context",
        "review",
        "knowledge:web/sessions",
        "--yes",
        "--json",
    ]);
    assert!(
        applied.status.success(),
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let applied: Value = serde_json::from_slice(&applied.stdout).unwrap();
    assert_eq!(applied["reference"], "knowledge:web/sessions");
    assert_eq!(f.observation("routing")["state"], "unreviewed");
}
