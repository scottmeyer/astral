#[cfg(not(unix))]
#[test]
fn inspection_explicitly_rejects_unsupported_platforms() {
    let error = ostk_gpt_cache::project::Project::load(".").err().unwrap();
    assert_eq!(error.code, "UNSUPPORTED_PLATFORM");
}

#[cfg(unix)]
mod unix {

    use ostk_gpt_cache::project::{Limits, Project};
    use serde_json::{Value, json};
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    fn put(root: &Path, path: &str, content: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn replace(root: &Path, path: &str, from: &str, to: &str) {
        let source = fs::read_to_string(root.join(path)).unwrap();
        assert!(source.contains(from), "replacement target {from:?} absent");
        put(root, path, &source.replace(from, to));
    }

    fn item(id: &str, deps: &[&str]) -> Value {
        json!({"schema_version": 1, "id": id, "title": format!("Implement {id}"),
        "status": "open", "depends_on": deps, "acceptance": ["Inspect without executing"]})
    }

    fn write_work(root: &Path, items: &[Value]) {
        put(
            root,
            ".astral/work/items.jsonl",
            &items.iter().map(|v| format!("{v}\n")).collect::<String>(),
        );
    }

    fn subsystem(root: &Path, id: &str, deps: &[&str]) {
        let deps = serde_json::to_string(deps).unwrap();
        put(
            root,
            &format!(".astral/core/{id}/subsystem.toml"),
            &format!(
                r#"schema_version = 1
id = "{id}"
purpose = "{id} implementation"
readme = "README.md"
rules = ["rules/required.md"]
decisions = ["decisions/initial.md"]
work_items = ["AST-001"]
projection = "saved"
depends_on = {deps}
"#
            ),
        );
        put(
            root,
            &format!(".astral/core/{id}/README.md"),
            &format!("{id} scoped instructions, not executed"),
        );
        put(
            root,
            &format!(".astral/core/{id}/rules/required.md"),
            "Preserve the active requirements.",
        );
        put(
            root,
            &format!(".astral/core/{id}/decisions/initial.md"),
            "Decision: inspect first.",
        );
    }

    fn fixture() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(
            root,
            ".astral/project.toml",
            r#"schema_version = 1
schema_status = "draft"
id = "test-project"
name = "Test project"
description = "A bounded fixture"
core = "core"
projections = "projections"
work_items = "work/items.jsonl"
[identity]
scope = "repository-local logical names"
runtime_bindings = "private and external to Git"
[subsystems]
web = "core/web"
authentication = "core/authentication"
other = "core/other"
"#,
        );
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(
                root,
                &format!(".astral/core/{name}"),
                "Shared project instructions.",
            );
        }
        write_work(root, &[item("AST-001", &["AST-002"]), item("AST-002", &[])]);
        subsystem(root, "web", &["authentication"]);
        subsystem(root, "authentication", &[]);
        subsystem(root, "other", &[]);
        put(
            root,
            ".astral/projections/saved/projection.toml",
            r#"schema_version = 1
id = "saved"
kind = "reviewable-design-context"
subsystems = ["web"]
handoff = "handoff.md"
native_payload_in_repository = false
[[sources]]
id = "private-history"
kind = "historical-native-conversation"
scope = "Past architecture discussion"
availability = "Private archive; unavailable in this checkout"
locator_policy = "resolve outside portable identity fields"
verification = "Historical only"
"#,
        );
        put(
            root,
            ".astral/projections/saved/handoff.md",
            "Readable context is not native state.",
        );
        dir
    }

    fn failure(root: &Path) -> String {
        match Project::load(root) {
            Ok(_) => panic!("expected invalid fixture"),
            Err(err) => err.code.to_owned(),
        }
    }

    fn inspect(root: &Path, name: &str) -> Value {
        Project::load(root)
            .unwrap()
            .inspect(name, Some("AST-001"))
            .unwrap()
    }

    #[test]
    fn validates_lists_and_resolves_explicit_scope_with_dependency_closure() {
        let root = fixture();
        let project = Project::load(root.path()).unwrap();
        assert_eq!(project.validate().unwrap()["status"], "valid");
        assert_eq!(
            project.list().unwrap()["contexts"]
                .as_array()
                .unwrap()
                .len(),
            4
        );
        let selected = project.inspect("web", Some("AST-001")).unwrap();
        assert_eq!(
            selected["selection"]["subsystems"],
            json!(["authentication", "web"])
        );
        assert_eq!(selected["native_binding"]["state"], "UNBOUND");
        assert_eq!(selected["native_binding"]["launch"], false);
        assert_eq!(selected["native_binding"]["plaintext_substitution"], false);
        assert_eq!(selected["work_item"]["id"], "AST-001");
        let sources = selected["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 13);
        assert!(
            sources
                .iter()
                .all(|s| !s["path"].as_str().unwrap().contains("other/"))
        );
        assert!(
            sources
                .iter()
                .all(|s| !s["path"].as_str().unwrap().contains("projections/"))
        );
        let record = sources
            .iter()
            .find(|s| s.get("record_id").is_some())
            .unwrap();
        assert_eq!(record["record_id"], "AST-001");
        assert_eq!(
            record["sha256"],
            ostk_gpt_cache::hash(item("AST-001", &["AST-002"]).to_string().as_bytes())
        );
        assert!(
            !selected
                .to_string()
                .contains("scoped instructions, not executed")
        );
    }

    #[test]
    fn projection_adds_handoff_and_inert_source_metadata_without_native_binding() {
        let root = fixture();
        let selected = inspect(root.path(), "projection:saved");
        assert_eq!(selected["selection"]["projection"], "saved");
        assert_eq!(
            selected["selection"]["subsystems"],
            json!(["authentication", "web"])
        );
        assert!(
            selected["sources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["path"] == ".astral/projections/saved/handoff.md")
        );
        assert_eq!(
            selected["metadata"]["projection"]["sources"][0]["availability"],
            "Private archive; unavailable in this checkout"
        );
        assert_eq!(selected["native_binding"]["state"], "UNBOUND");
    }

    #[test]
    fn digest_is_independent_of_machine_root_and_changes_on_selected_edits() {
        let left = fixture();
        let right = fixture();
        let before = inspect(left.path(), "web");
        assert_eq!(before, inspect(right.path(), "subsystem:web"));
        put(
            right.path(),
            ".astral/core/web/README.md",
            "Changed selected context",
        );
        assert_ne!(
            before["selection_digest"],
            inspect(right.path(), "web")["selection_digest"]
        );
    }

    #[test]
    fn unrelated_docs_and_work_records_do_not_pollute_selected_fingerprint() {
        let root = fixture();
        let before = inspect(root.path(), "web");
        put(
            root.path(),
            ".astral/core/other/README.md",
            "Unrelated subsystem edit",
        );
        let mut unrelated = item("AST-002", &[]);
        unrelated["title"] = json!("Changed unrelated task");
        write_work(root.path(), &[item("AST-001", &["AST-002"]), unrelated]);
        assert_eq!(before, inspect(root.path(), "web"));
        let mut selected = item("AST-001", &["AST-002"]);
        selected["acceptance"] = json!(["New exact requirement"]);
        write_work(root.path(), &[selected, item("AST-002", &[])]);
        assert_ne!(
            before["selection_digest"],
            inspect(root.path(), "web")["selection_digest"]
        );
    }

    #[test]
    fn no_work_selection_omits_record_and_missing_work_errors() {
        let root = fixture();
        let project = Project::load(root.path()).unwrap();
        let no_work = project.inspect("web", None).unwrap();
        assert!(no_work["work_item"].is_null());
        assert!(
            !no_work["sources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s.get("record_id").is_some())
        );
        assert_eq!(
            project.inspect("web", Some("MISSING")).unwrap_err().code,
            "UNKNOWN_WORK_ITEM"
        );
    }

    #[test]
    fn malformed_jsonl_blank_records_and_duplicate_ids_are_rejected() {
        for malformed in ["{broken}\n", "\n", "{\"schema_version\":1}\n"] {
            let root = fixture();
            put(root.path(), ".astral/work/items.jsonl", malformed);
            assert_eq!(failure(root.path()), "INVALID_WORK_ITEM");
        }
        let root = fixture();
        write_work(root.path(), &[item("AST-001", &[]), item("AST-001", &[])]);
        assert_eq!(failure(root.path()), "DUPLICATE_ID");
    }

    #[test]
    fn strict_work_types_status_and_versions() {
        for (field, value, expected) in [
            ("schema_version", json!(2), "UNSUPPORTED_SCHEMA"),
            ("schema_version", json!("1"), "INVALID_WORK_ITEM"),
            ("status", json!("done"), "INVALID_WORK_ITEM"),
            ("acceptance", json!([]), "INVALID_FIELD"),
            ("acceptance", json!("must be array"), "INVALID_WORK_ITEM"),
            ("title", json!(" "), "INVALID_FIELD"),
            ("evidence", json!(["../escape"]), "UNSAFE_PATH"),
            ("unknown", json!(true), "INVALID_WORK_ITEM"),
        ] {
            let root = fixture();
            let mut changed = item("AST-001", &[]);
            changed[field] = value;
            write_work(root.path(), &[changed]);
            assert_eq!(failure(root.path()), expected, "field {field}");
        }
    }

    #[test]
    fn rejects_unknown_manifest_fields_versions_and_registry_mismatches() {
        for (path, from, to, expected) in [
            (
                ".astral/project.toml",
                "schema_version = 1",
                "schema_version = 2",
                "UNSUPPORTED_SCHEMA",
            ),
            (
                ".astral/core/web/subsystem.toml",
                "schema_version = 1",
                "schema_version = 0",
                "UNSUPPORTED_SCHEMA",
            ),
            (
                ".astral/projections/saved/projection.toml",
                "schema_version = 1",
                "schema_version = 999",
                "UNSUPPORTED_SCHEMA",
            ),
            (
                ".astral/project.toml",
                "schema_version = 1",
                "schema_version = 1\nunknown = true",
                "INVALID_MANIFEST",
            ),
            (
                ".astral/core/web/subsystem.toml",
                "id = \"web\"",
                "id = \"other-id\"",
                "REGISTRY_MISMATCH",
            ),
            (
                ".astral/projections/saved/projection.toml",
                "id = \"saved\"",
                "id = \"other-id\"",
                "REGISTRY_MISMATCH",
            ),
            (
                ".astral/projections/saved/projection.toml",
                "native_payload_in_repository = false",
                "native_payload_in_repository = true",
                "INVALID_NATIVE_REFERENCE",
            ),
        ] {
            let root = fixture();
            replace(root.path(), path, from, to);
            assert_eq!(failure(root.path()), expected, "{path}");
        }
    }

    #[test]
    fn missing_references_and_cycles_fail_for_both_graphs() {
        for (deps, expected) in [
            (vec!["missing"], "MISSING_REFERENCE"),
            (vec!["AST-001"], "DEPENDENCY_CYCLE"),
        ] {
            let root = fixture();
            write_work(root.path(), &[item("AST-001", &deps), item("AST-002", &[])]);
            assert_eq!(failure(root.path()), expected);
        }
        let root = fixture();
        write_work(
            root.path(),
            &[item("AST-001", &["AST-002"]), item("AST-002", &["AST-001"])],
        );
        assert_eq!(failure(root.path()), "DEPENDENCY_CYCLE");
        for (deps, expected) in [
            (vec!["missing"], "MISSING_REFERENCE"),
            (vec!["web"], "DEPENDENCY_CYCLE"),
        ] {
            let root = fixture();
            subsystem(root.path(), "authentication", &deps);
            assert_eq!(failure(root.path()), expected);
        }
        for (path, from, to) in [
            (
                ".astral/core/web/subsystem.toml",
                "projection = \"saved\"",
                "projection = \"missing\"",
            ),
            (
                ".astral/core/web/subsystem.toml",
                "work_items = [\"AST-001\"]",
                "work_items = [\"missing\"]",
            ),
            (
                ".astral/projections/saved/projection.toml",
                "subsystems = [\"web\"]",
                "subsystems = [\"missing\"]",
            ),
        ] {
            let root = fixture();
            replace(root.path(), path, from, to);
            assert_eq!(failure(root.path()), "MISSING_REFERENCE");
        }
    }

    #[test]
    fn namespace_collision_requires_explicit_selector() {
        let root = fixture();
        fs::rename(
            root.path().join(".astral/projections/saved"),
            root.path().join(".astral/projections/web"),
        )
        .unwrap();
        replace(
            root.path(),
            ".astral/projections/web/projection.toml",
            "id = \"saved\"",
            "id = \"web\"",
        );
        for id in ["authentication", "web", "other"] {
            replace(
                root.path(),
                &format!(".astral/core/{id}/subsystem.toml"),
                "projection = \"saved\"",
                "projection = \"web\"",
            );
        }
        let project = Project::load(root.path()).unwrap();
        assert_eq!(
            project.inspect("web", None).unwrap_err().code,
            "AMBIGUOUS_CONTEXT"
        );
        assert_eq!(
            project.inspect("subsystem:web", None).unwrap()["selection"]["kind"],
            "subsystem"
        );
        assert_eq!(
            project.inspect("projection:web", None).unwrap()["selection"]["kind"],
            "projection"
        );
        assert_eq!(
            project.inspect("unknown:web", None).unwrap_err().code,
            "INVALID_SELECTOR"
        );
        assert_eq!(
            project.inspect("subsystem:missing", None).unwrap_err().code,
            "UNKNOWN_CONTEXT"
        );
    }

    #[test]
    fn rejects_traversal_absolute_windows_and_noncanonical_paths() {
        for path in [
            "../escape.md",
            "/tmp/escape.md",
            "rules/../../escape.md",
            "C:/escape.md",
            r"C:\escape.md",
            r"..\escape.md",
            "rules//required.md",
            "./README.md",
            "",
        ] {
            let root = fixture();
            // TOML literal strings preserve backslashes for the validator to see.
            replace(
                root.path(),
                ".astral/core/web/subsystem.toml",
                "readme = \"README.md\"",
                &format!("readme = '{path}'"),
            );
            assert_eq!(failure(root.path()), "UNSAFE_PATH", "{path:?}");
        }
        let root = fixture();
        replace(
            root.path(),
            ".astral/project.toml",
            "core = \"core\"",
            "core = \"../core\"",
        );
        assert_eq!(failure(root.path()), "UNSAFE_PATH");
    }

    #[test]
    fn rejects_symlinks_at_astral_intermediate_leaf_and_discovery() {
        use std::os::unix::fs::symlink;
        for path in [
            ".astral",
            ".astral/core/web",
            ".astral/core/web/README.md",
            ".astral/projections",
            ".astral/projections/saved",
        ] {
            let root = fixture();
            let original = root.path().join(path);
            let target = root.path().join("moved");
            fs::rename(&original, &target).unwrap();
            symlink(&target, &original).unwrap();
            assert_eq!(failure(root.path()), "SYMLINK_REJECTED", "{path}");
        }
    }

    #[test]
    fn rejects_nonregular_files_before_reading_or_blocking() {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let root = fixture();
        let readme = root.path().join(".astral/core/web/README.md");
        fs::remove_file(&readme).unwrap();
        let name = CString::new(readme.as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert_eq!(failure(root.path()), "INVALID_FILE_TYPE");
        fs::remove_file(&readme).unwrap();
        fs::create_dir(&readme).unwrap();
        assert_eq!(failure(root.path()), "INVALID_FILE_TYPE");
    }

    #[test]
    fn bounds_file_total_entries_files_depth_and_output() {
        let root = fixture();
        for limits in [
            Limits {
                file_bytes: 8,
                ..Limits::default()
            },
            Limits {
                total_bytes: 8,
                ..Limits::default()
            },
            Limits {
                files: 1,
                ..Limits::default()
            },
            Limits {
                entries: 1,
                ..Limits::default()
            },
            Limits {
                graph_depth: 1,
                ..Limits::default()
            },
        ] {
            let error = match Project::load_with_limits(root.path(), limits) {
                Ok(_) => panic!("limit must fail"),
                Err(error) => error,
            };
            assert_eq!(error.code, "LIMIT_EXCEEDED", "{limits:?}");
        }
        let project = Project::load_with_limits(
            root.path(),
            Limits {
                output_bytes: 16,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(
            project.inspect("web", None).unwrap_err().code,
            "LIMIT_EXCEEDED"
        );
        assert_eq!(project.list().unwrap_err().code, "LIMIT_EXCEEDED");
        assert_eq!(project.validate().unwrap_err().code, "LIMIT_EXCEEDED");
        put(
            root.path(),
            ".astral/core/web/README.md",
            &"x".repeat(Limits::default().file_bytes + 1),
        );
        assert_eq!(failure(root.path()), "LIMIT_EXCEEDED");
    }

    #[test]
    fn missing_files_and_invalid_directory_entries_are_not_silently_ignored() {
        let root = fixture();
        fs::remove_file(root.path().join(".astral/core/RUN.md")).unwrap();
        assert_eq!(failure(root.path()), "PATH_UNAVAILABLE");
        let root = fixture();
        put(
            root.path(),
            ".astral/projections/not-a-directory",
            "ignored? no",
        );
        assert_eq!(failure(root.path()), "INVALID_FILE_TYPE");
    }

    #[test]
    fn cli_inspects_only_and_returns_json_for_success_and_failure() {
        let root = fixture();
        let binary = env!("CARGO_BIN_EXE_astral");
        let output = Command::new(binary)
            .args([
                "--root",
                root.path().to_str().unwrap(),
                "project",
                "web",
                "--inspect",
                "--work",
                "AST-001",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(json["native_binding"]["launch"], false);
        assert!(output.stderr.is_empty());
        let output = Command::new(binary)
            .args(["--root", "/nonexistent", "project", "web", "--proxy"])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
            "INVALID_ROOT"
        );
        for args in [["context", "validate"], ["context", "list"]] {
            let output = Command::new(binary)
                .current_dir(root.path())
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success());
            serde_json::from_slice::<Value>(&output.stdout).unwrap();
        }
        let output = Command::new(binary).arg("--unknown").output().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&output.stderr).unwrap()["error"]["code"],
            "CLI_USAGE"
        );
    }

    #[test]
    fn malformed_source_cannot_expand_error_output_without_bound() {
        let root = fixture();
        replace(
            root.path(),
            ".astral/core/web/subsystem.toml",
            "readme = \"README.md\"",
            &format!("readme = \"{}\"", "x".repeat(32_768)),
        );
        let error = Project::load(root.path()).err().unwrap();
        assert_eq!(error.code, "PATH_UNAVAILABLE");
        assert!(error.message.len() <= 512);
        let output = Command::new(env!("CARGO_BIN_EXE_astral"))
            .args([
                "--root",
                root.path().to_str().unwrap(),
                "context",
                "validate",
            ])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stderr.len() < 4096);
        serde_json::from_slice::<Value>(&output.stderr).unwrap();
    }

    #[test]
    fn fresh_context_contains_only_selected_readable_documents_and_work() {
        let root = fixture();
        let project = Project::load(root.path()).unwrap();
        let inspected = project.inspect("web", Some("AST-001")).unwrap();
        let fresh = project.fresh_context("web", Some("AST-001")).unwrap();
        assert_eq!(fresh.selection.subsystems, ["authentication", "web"]);
        assert_eq!(fresh.documents.len(), 9);
        assert!(
            fresh
                .documents
                .iter()
                .any(|doc| doc.source.path == ".astral/core/ARCHITECTURE.md")
        );
        assert!(
            fresh
                .documents
                .iter()
                .any(|doc| doc.source.path == ".astral/core/authentication/README.md")
        );
        assert!(
            fresh
                .documents
                .iter()
                .any(|doc| doc.text == "web scoped instructions, not executed")
        );
        assert!(
            fresh
                .documents
                .iter()
                .all(|doc| !doc.source.path.contains("other/")
                    && !doc.source.path.ends_with(".toml")
                    && !doc.source.path.contains("projections/"))
        );
        let work = fresh.work_item.as_ref().unwrap();
        assert_eq!(work.item.id, "AST-001");
        assert_eq!(work.source.record_id.as_deref(), Some("AST-001"));
        assert!(
            !fresh
                .sources
                .iter()
                .any(|source| source.record_id.as_deref() == Some("AST-002"))
        );
        for doc in &fresh.documents {
            assert_eq!(doc.source.bytes, doc.text.len());
            assert_eq!(doc.source.sha256, ostk_gpt_cache::hash(doc.text.as_bytes()));
        }
        assert_eq!(work.source.bytes, work.text.len());
        assert_eq!(
            work.source.sha256,
            ostk_gpt_cache::hash(work.text.as_bytes())
        );
        assert_eq!(project.inspect("web", Some("AST-001")).unwrap(), inspected);
        assert!(
            project
                .fresh_context("web", None)
                .unwrap()
                .work_item
                .is_none()
        );

        let explicit = project.fresh_context("projection:saved", None).unwrap();
        assert!(explicit.documents.iter().any(|doc| doc.source.path
            == ".astral/projections/saved/handoff.md"
            && doc.text == "Readable context is not native state."));
        assert_eq!(explicit.documents.len(), 10);
    }

    #[test]
    fn fresh_context_reuses_observed_document_and_exact_work_bytes_after_edits() {
        let root = fixture();
        let exact = format!("  {}  ", item("AST-001", &["AST-002"]));
        put(
            root.path(),
            ".astral/work/items.jsonl",
            &format!("{exact}\r\n{}\r\n", item("AST-002", &[])),
        );
        let project = Project::load(root.path()).unwrap();
        let before = project.fresh_context("web", Some("AST-001")).unwrap();
        assert_eq!(before.work_item.as_ref().unwrap().text, exact);
        let inspection_before = project.inspect("web", Some("AST-001")).unwrap();
        put(
            root.path(),
            ".astral/core/web/README.md",
            "Changed after load",
        );
        put(
            root.path(),
            ".astral/core/ARCHITECTURE.md",
            "New architecture after load",
        );
        replace(
            root.path(),
            ".astral/project.toml",
            "name = \"Test project\"",
            "name = \"Different project name\"",
        );
        let mut changed_work = item("AST-001", &["AST-002"]);
        changed_work["title"] = json!("Changed work after load");
        write_work(root.path(), &[changed_work, item("AST-002", &[])]);
        let after = project.fresh_context("web", Some("AST-001")).unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert_eq!(
            inspection_before,
            project.inspect("web", Some("AST-001")).unwrap()
        );
        let reloaded = Project::load(root.path())
            .unwrap()
            .fresh_context("web", Some("AST-001"))
            .unwrap();
        assert_ne!(before.selection_digest, reloaded.selection_digest);
        assert_eq!(before.work_item.unwrap().item.title, "Implement AST-001");
    }

    #[test]
    fn fresh_rejects_selected_invalid_utf8_without_reading_unselected_text_into_context() {
        let root = fixture();
        fs::write(root.path().join(".astral/core/other/README.md"), [0xff]).unwrap();
        let project = Project::load(root.path()).unwrap();
        assert!(project.fresh_context("web", None).is_ok());
        assert_eq!(
            project.fresh_context("other", None).unwrap_err().code,
            "INVALID_UTF8"
        );
        assert!(project.inspect("other", None).is_ok());
        for (path, selected) in [
            (".astral/core/web/README.md", "web"),
            (".astral/core/RUN.md", "web"),
            (".astral/projections/saved/handoff.md", "saved"),
        ] {
            let root = fixture();
            fs::write(root.path().join(path), [0xff, 0xfe]).unwrap();
            let project = Project::load(root.path()).unwrap();
            assert_eq!(
                project.fresh_context(selected, None).unwrap_err().code,
                "INVALID_UTF8"
            );
        }
    }

    #[test]
    fn fresh_bounds_raw_text_and_serialized_expansion() {
        let root = fixture();
        let small = Project::load_with_limits(
            root.path(),
            Limits {
                output_bytes: 64,
                ..Limits::default()
            },
        )
        .unwrap();
        assert_eq!(
            small.fresh_context("web", None).unwrap_err().code,
            "LIMIT_EXCEEDED"
        );
        put(
            root.path(),
            ".astral/core/web/README.md",
            &"\u{0001}".repeat(60_000),
        );
        let project = Project::load_with_limits(
            root.path(),
            Limits {
                output_bytes: 200_000,
                ..Limits::default()
            },
        )
        .unwrap();
        assert!(project.inspect("web", None).is_ok());
        let error = project.fresh_context("web", None).unwrap_err();
        assert_eq!(error.code, "LIMIT_EXCEEDED");
        assert_eq!(error.message, "fresh context output byte limit");
    }

    #[test]
    fn fresh_never_falls_back_from_native_or_unknown_projection_kinds() {
        for kind in ["native-checkpoint", "native", "future-unknown-kind"] {
            let root = fixture();
            replace(
                root.path(),
                ".astral/projections/saved/projection.toml",
                "kind = \"reviewable-design-context\"",
                &format!("kind = \"{kind}\""),
            );
            if kind == "native-checkpoint" {
                assert_eq!(failure(root.path()), "MISSING_NATIVE_BUNDLE");
                continue;
            }
            let project = Project::load(root.path()).unwrap();
            assert!(project.inspect("saved", None).is_ok());
            for selector in ["saved", "web", "authentication"] {
                assert_eq!(
                    project.fresh_context(selector, None).unwrap_err().code,
                    "UNSUPPORTED_FRESH_PROJECTION"
                );
            }
        }
        for kind in ["reviewable-design-context", "reviewable", "fresh-context"] {
            let root = fixture();
            replace(
                root.path(),
                ".astral/projections/saved/projection.toml",
                "kind = \"reviewable-design-context\"",
                &format!("kind = \"{kind}\""),
            );
            assert!(
                Project::load(root.path())
                    .unwrap()
                    .fresh_context("saved", None)
                    .is_ok()
            );
        }
        let root = fixture();
        replace(
            root.path(),
            ".astral/projections/saved/projection.toml",
            "native_payload_in_repository = false",
            "native_payload_in_repository = true",
        );
        assert_eq!(failure(root.path()), "INVALID_NATIVE_REFERENCE");
        let root = fixture();
        replace(
            root.path(),
            ".astral/projections/saved/projection.toml",
            "native_payload_in_repository = false",
            "native_payload_in_repository = false\nnative_checkpoint = 'private external artifact'",
        );
        assert_eq!(failure(root.path()), "INVALID_MANIFEST");
    }

    #[test]
    fn unknown_native_linked_projection_of_a_selected_dependency_is_not_bypassed() {
        let root = fixture();
        let saved = fs::read_to_string(
            root.path()
                .join(".astral/projections/saved/projection.toml"),
        )
        .unwrap();
        put(
            root.path(),
            ".astral/projections/native/projection.toml",
            &saved
                .replace("id = \"saved\"", "id = \"native\"")
                .replace("kind = \"reviewable-design-context\"", "kind = \"native\""),
        );
        put(
            root.path(),
            ".astral/projections/native/handoff.md",
            "Do not substitute this for a native checkpoint.",
        );
        replace(
            root.path(),
            ".astral/core/authentication/subsystem.toml",
            "projection = \"saved\"",
            "projection = \"native\"",
        );
        let project = Project::load(root.path()).unwrap();
        assert_eq!(
            project.fresh_context("web", None).unwrap_err().code,
            "UNSUPPORTED_FRESH_PROJECTION"
        );
        assert_eq!(
            project.fresh_context("saved", None).unwrap_err().code,
            "UNSUPPORTED_FRESH_PROJECTION"
        );
        assert!(project.fresh_context("other", None).is_ok());
    }

    #[test]
    fn fresh_preserves_existing_resolution_errors() {
        let root = fixture();
        let project = Project::load(root.path()).unwrap();
        for (selector, work, code) in [
            ("missing", None, "UNKNOWN_CONTEXT"),
            ("web", Some("missing"), "UNKNOWN_WORK_ITEM"),
            ("unknown:web", None, "INVALID_SELECTOR"),
        ] {
            assert_eq!(
                project.fresh_context(selector, work).unwrap_err().code,
                code
            );
        }
    }

    #[test]
    fn fresh_fingerprints_linked_projection_gate_without_changing_inspection() {
        let root = fixture();
        let project = Project::load(root.path()).unwrap();
        let before = project.fresh_context("web", None).unwrap();
        let inspected = project.inspect("web", None).unwrap();
        assert!(
            before
                .sources
                .iter()
                .any(|source| source.path == ".astral/projections/saved/projection.toml")
        );
        assert!(
            !before
                .documents
                .iter()
                .any(|document| document.source.path.contains("projections/"))
        );
        replace(
            root.path(),
            ".astral/projections/saved/projection.toml",
            "kind = \"reviewable-design-context\"",
            "kind = \"fresh-context\"",
        );
        let reloaded = Project::load(root.path()).unwrap();
        assert_ne!(
            before.selection_digest,
            reloaded
                .fresh_context("web", None)
                .unwrap()
                .selection_digest
        );
        assert_eq!(inspected, reloaded.inspect("web", None).unwrap());
        assert_eq!(
            serde_json::to_value(before).unwrap(),
            serde_json::to_value(project.fresh_context("web", None).unwrap()).unwrap()
        );
    }
}
