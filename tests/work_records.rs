use astral::project::WorkStatus;
use astral::work_records::{self, ConflictKind, MAX_FILE_BYTES, MergeResult, parse};
use serde_json::{Value, json};

fn item(id: &str, dependencies: &[&str]) -> Value {
    json!({
        "schema_version": 1, "id": id, "title": format!("Implement {id}"),
        "status": "open", "depends_on": dependencies,
        "acceptance": ["Preserve existing work"],
        "evidence": ["docs/receipt.md"], "provenance": "Historical design",
        "scope": "Local records", "result": "Not yet checked", "remaining": "Implementation"
    })
}

fn jsonl(items: &[Value]) -> Vec<u8> {
    items
        .iter()
        .map(|item| format!("{item}\n"))
        .collect::<String>()
        .into_bytes()
}

#[test]
fn cli_merge_returns_usable_jsonl_and_nonzero_explicit_conflicts_without_editing_inputs() {
    let temp = tempfile::tempdir().unwrap();
    let paths = [
        temp.path().join("base"),
        temp.path().join("ours"),
        temp.path().join("theirs"),
    ];
    let original = item("AST-001", &[]);
    let extra = item("AST-002", &[]);
    for (path, bytes) in paths.iter().zip([
        jsonl(std::slice::from_ref(&original)),
        jsonl(&[original.clone(), extra]),
        jsonl(std::slice::from_ref(&original)),
    ]) {
        std::fs::write(path, bytes).unwrap();
    }
    let invoke = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_astral"))
            .args(["--json", "work", "merge", "--base"])
            .arg(&paths[0])
            .arg("--ours")
            .arg(&paths[1])
            .arg("--theirs")
            .arg(&paths[2])
            .output()
            .unwrap()
    };
    let success = invoke();
    assert!(success.status.success());
    let output: Value = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(
        parse(output["jsonl"].as_str().unwrap().as_bytes())
            .unwrap()
            .len(),
        2
    );
    let mut left = original.clone();
    left["status"] = json!("in_progress");
    let mut right = original;
    right["status"] = json!("complete");
    std::fs::write(&paths[1], jsonl(&[left])).unwrap();
    std::fs::write(&paths[2], jsonl(&[right])).unwrap();
    let before: Vec<_> = paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    let conflict = invoke();
    assert_eq!(conflict.status.code(), Some(1));
    let output: Value = serde_json::from_slice(&conflict.stdout).unwrap();
    assert_eq!(output["outcome"], "conflicts");
    assert!(output.get("jsonl").is_none());
    assert_eq!(
        before,
        paths
            .iter()
            .map(|p| std::fs::read(p).unwrap())
            .collect::<Vec<_>>()
    );
}

fn merged(base: &[Value], ours: &[Value], theirs: &[Value]) -> (Vec<u8>, Vec<String>) {
    match work_records::merge(&jsonl(base), &jsonl(ours), &jsonl(theirs)).unwrap() {
        MergeResult::Merged { bytes, records } => (
            bytes,
            records.into_iter().map(|record| record.item.id).collect(),
        ),
        result => panic!("expected successful merge, got {result:?}"),
    }
}

fn conflict(base: &[Value], ours: &[Value], theirs: &[Value], expected: ConflictKind) {
    for (ours, theirs) in [(ours, theirs), (theirs, ours)] {
        let result = work_records::merge(&jsonl(base), &jsonl(ours), &jsonl(theirs)).unwrap();
        match &result {
            MergeResult::Conflicts { conflicts } => {
                assert_eq!(conflicts.len(), 1);
                assert_eq!(conflicts[0].id, "AST-001");
                assert_eq!(conflicts[0].kind, expected);
            }
            _ => panic!("expected conflict"),
        }
        let diagnostic = serde_json::to_string(&result).unwrap();
        assert!(!diagnostic.contains("private"));
        assert!(!diagnostic.contains("title"));
        assert!(!diagnostic.contains("bytes"));
    }
}

#[test]
fn parsing_reuses_models_retains_historical_ids_and_hashes_exact_lines() {
    let first = item("legacy-ticket", &[]);
    let second = item("AST-001", &["legacy-ticket"]);
    let bytes = format!(" {first} \r\n{second}\r\n");
    let records = parse(bytes.as_bytes()).unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].item.id, "AST-001");
    assert_eq!(records[1].item.id, "legacy-ticket");
    assert_eq!(
        records[1].digest,
        astral::hash(format!(" {first} ").as_bytes())
    );
    assert_eq!(serde_json::to_value(&records[0].item).unwrap(), second);
    assert!(parse(b"").unwrap().is_empty());
}

#[test]
fn strict_schema_and_nonempty_fields_are_enforced_without_echoing_rows() {
    let base = item("AST-001", &[]);
    for (field, value) in [
        ("unexpected", json!("private source")),
        ("status", json!("deleted")),
        ("schema_version", json!(2)),
        ("title", json!(" \t")),
        ("acceptance", json!([])),
        ("acceptance", json!([""])),
        ("acceptance", json!(["same", "same"])),
        ("depends_on", json!(["AST-001", "AST-001"])),
        ("id", json!("../outside")),
        ("evidence", json!(["../private"])),
    ] {
        let mut invalid = base.clone();
        invalid[field] = value;
        invalid["provenance"] = json!("private source");
        let error = parse(&jsonl(&[invalid])).unwrap_err();
        assert!(!error.to_string().contains("private"));
        assert!(!serde_json::to_string(&error).unwrap().contains("private"));
    }
    for field in [
        "schema_version",
        "id",
        "title",
        "status",
        "depends_on",
        "acceptance",
    ] {
        let mut invalid = base.clone();
        invalid.as_object_mut().unwrap().remove(field);
        assert_eq!(
            parse(&jsonl(&[invalid])).unwrap_err().code,
            "INVALID_WORK_ITEM"
        );
    }
    let duplicate_field =
        String::from_utf8(jsonl(&[base]))
            .unwrap()
            .replacen('{', "{\"id\":\"AST-001\",", 1);
    assert_eq!(
        parse(duplicate_field.as_bytes()).unwrap_err().code,
        "INVALID_WORK_ITEM"
    );
    for invalid in [
        b"\n".as_slice(),
        b"# comment\n",
        b"{}\n",
        b"\xff\n",
        b"[]\n",
    ] {
        assert!(parse(invalid).is_err());
    }
}

#[test]
fn parsing_rejects_duplicate_ids_missing_references_cycles_and_depth() {
    let duplicate = item("AST-001", &[]);
    assert_eq!(
        parse(&jsonl(&[duplicate.clone(), duplicate]))
            .unwrap_err()
            .code,
        "DUPLICATE_ID"
    );
    let error = parse(&jsonl(&[item("AST-001", &["AST-002"])])).unwrap_err();
    assert_eq!(error.code, "MISSING_REFERENCE");
    assert_eq!(error.ids, ["AST-001", "AST-002"]);
    assert_eq!(
        parse(&jsonl(&[item("AST-001", &["AST-001"])]))
            .unwrap_err()
            .code,
        "DEPENDENCY_CYCLE"
    );
    assert_eq!(
        parse(&jsonl(&[
            item("AST-001", &["AST-002"]),
            item("AST-002", &["AST-001"])
        ]))
        .unwrap_err()
        .code,
        "DEPENDENCY_CYCLE"
    );
    let chain: Vec<_> = (0..65)
        .map(|n| {
            if n == 64 {
                item(&format!("item-{n:03}"), &[])
            } else {
                item(&format!("item-{n:03}"), &[&format!("item-{:03}", n + 1)])
            }
        })
        .collect();
    assert!(parse(&jsonl(&chain[1..])).is_ok());
    assert_eq!(parse(&jsonl(&chain)).unwrap_err().code, "LIMIT_EXCEEDED");
}

#[test]
fn byte_record_and_aggregate_entry_limits_are_bounded() {
    assert_eq!(
        parse(&vec![b' '; MAX_FILE_BYTES + 1]).unwrap_err().code,
        "LIMIT_EXCEEDED"
    );
    let compact = b"{\"schema_version\":1,\"id\":\"x\",\"title\":\"x\",\"status\":\"open\",\"depends_on\":[],\"acceptance\":[\"x\"]}\n";
    let too_many = compact.repeat(work_records::MAX_RECORDS + 1);
    assert!(too_many.len() < MAX_FILE_BYTES);
    assert_eq!(parse(&too_many).unwrap_err().code, "LIMIT_EXCEEDED");
    let mut oversized_entries = item("AST-001", &[]);
    oversized_entries["acceptance"] = json!(
        (0..work_records::MAX_ENTRIES)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        parse(&jsonl(&[oversized_entries])).unwrap_err().code,
        "LIMIT_EXCEEDED"
    );
    let mut maximum_bytes = item("AST-001", &[]);
    let current = jsonl(&[maximum_bytes.clone()]).len();
    maximum_bytes["title"] = json!(
        "x".repeat(MAX_FILE_BYTES - current + maximum_bytes["title"].as_str().unwrap().len())
    );
    assert_eq!(jsonl(&[maximum_bytes.clone()]).len(), MAX_FILE_BYTES);
    assert!(parse(&jsonl(&[maximum_bytes])).is_ok());
}

#[test]
fn merge_unions_independent_additions_and_is_deterministic() {
    let historical = item("AST-001", &[]);
    let a = item("AST-abcdefghjkmn", &["AST-001"]);
    let b = item("AST-0123456789ab", &["AST-001"]);
    let (bytes, ids) = merged(
        std::slice::from_ref(&historical),
        &[a.clone(), historical.clone()],
        &[historical.clone(), b.clone()],
    );
    assert_eq!(ids, ["AST-001", "AST-0123456789ab", "AST-abcdefghjkmn"]);
    let (reverse, _) = merged(
        std::slice::from_ref(&historical),
        &[b, historical.clone()],
        &[historical.clone(), a],
    );
    assert_eq!(bytes, reverse);
    assert!(bytes.ends_with(b"\n"));
    assert_eq!(parse(&bytes).unwrap().len(), 3);
}

#[test]
fn identical_edits_and_formatting_only_changes_merge_semantically() {
    let original = item("AST-001", &[]);
    let mut changed = original.clone();
    changed["status"] = json!("complete");
    let (bytes, _) = merged(
        std::slice::from_ref(&original),
        std::slice::from_ref(&changed),
        std::slice::from_ref(&changed),
    );
    assert!(matches!(
        parse(&bytes).unwrap()[0].item.status,
        WorkStatus::Complete
    ));
    let reordered = serde_json::to_string(
        &serde_json::from_value::<astral::project::WorkItem>(original.clone()).unwrap(),
    )
    .unwrap();
    assert_ne!(
        reordered.as_bytes(),
        &jsonl(std::slice::from_ref(&original))[..reordered.len()]
    );
    let formatted = format!("  {reordered}  \r\n");
    let result = work_records::merge(
        &jsonl(&[original]),
        formatted.as_bytes(),
        &jsonl(&[changed]),
    )
    .unwrap();
    assert!(
        matches!(result, MergeResult::Merged { records, .. } if matches!(records[0].item.status, WorkStatus::Complete))
    );
    let (added, ids) = merged(&[], &[item("AST-001", &[])], &[item("AST-001", &[])]);
    assert_eq!(ids, ["AST-001"]);
    assert_eq!(parse(&added).unwrap().len(), 1);
}

#[test]
fn unchanged_vs_edit_and_deletion_use_the_changed_side() {
    let original = item("AST-001", &[]);
    let other = item("AST-002", &[]);
    let mut changed = original.clone();
    changed["status"] = json!("in_progress");
    for (ours, theirs) in [
        (vec![changed.clone()], vec![original.clone()]),
        (vec![original.clone()], vec![changed.clone()]),
    ] {
        let (bytes, _) = merged(std::slice::from_ref(&original), &ours, &theirs);
        assert!(matches!(
            parse(&bytes).unwrap()[0].item.status,
            WorkStatus::InProgress
        ));
    }
    let base = [original.clone(), other.clone()];
    for (ours, theirs) in [
        (vec![other.clone()], base.to_vec()),
        (base.to_vec(), vec![other.clone()]),
        (vec![other.clone()], vec![other.clone()]),
    ] {
        assert_eq!(merged(&base, &ours, &theirs).1, ["AST-002"]);
    }
    assert!(merged(&[original], &[], &[]).0.is_empty());
}

#[test]
fn differing_edits_additions_and_delete_modify_have_structured_conflicts() {
    let base = item("AST-001", &[]);
    let mut ours = base.clone();
    let mut theirs = base.clone();
    ours["title"] = json!("private ours");
    theirs["title"] = json!("private theirs");
    conflict(
        std::slice::from_ref(&base),
        std::slice::from_ref(&ours),
        std::slice::from_ref(&theirs),
        ConflictKind::DivergentEdit,
    );
    conflict(
        &[],
        std::slice::from_ref(&ours),
        std::slice::from_ref(&theirs),
        ConflictKind::ConcurrentAddition,
    );
    conflict(&[base], &[ours], &[], ConflictKind::ModifyDelete);
}

#[test]
fn merge_revalidates_dangling_dependencies_and_new_cycles() {
    let a = item("AST-001", &[]);
    let b = item("AST-002", &[]);
    let needs_b = item("AST-001", &["AST-002"]);
    let needs_a = item("AST-002", &["AST-001"]);
    let base = jsonl(&[a.clone(), b.clone()]);
    let dangling = work_records::merge(
        &base,
        &jsonl(std::slice::from_ref(&a)),
        &jsonl(&[needs_b.clone(), b.clone()]),
    )
    .unwrap_err();
    assert_eq!(dangling.code, "MISSING_REFERENCE");
    assert_eq!(dangling.ids, ["AST-001", "AST-002"]);
    let cyclic =
        work_records::merge(&base, &jsonl(&[needs_b, b]), &jsonl(&[a, needs_a])).unwrap_err();
    assert_eq!(cyclic.code, "DEPENDENCY_CYCLE");
    assert!(!cyclic.ids.is_empty());
}

#[test]
fn merged_output_must_also_fit_limits() {
    let mut a = item("AST-001", &[]);
    let mut b = item("AST-002", &[]);
    a["title"] = json!("a".repeat(MAX_FILE_BYTES / 2));
    b["title"] = json!("b".repeat(MAX_FILE_BYTES / 2));
    assert_eq!(
        work_records::merge(b"", &jsonl(&[a]), &jsonl(&[b]))
            .unwrap_err()
            .code,
        "LIMIT_EXCEEDED"
    );
}

#[cfg(unix)]
mod persistence {
    use super::*;
    use astral::project::Project;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    fn put(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    fn fixture() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        put(
            dir.path(),
            ".astral/project.toml",
            "schema_version=1\nid='fixture'\nname='Fixture'\ndescription='Test'\ncore='core'\nprojections='projections'\nwork_items='tasks/register.jsonl'\n[identity]\nscope='local'\nruntime_bindings='private'\n[subsystems]\n",
        );
        for file in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(dir.path(), &format!(".astral/core/{file}"), "Inert context");
        }
        fs::create_dir(dir.path().join(".astral/projections")).unwrap();
        put(
            dir.path(),
            ".astral/tasks/register.jsonl",
            jsonl(&[item("AST-001", &[])]),
        );
        Project::load(dir.path()).unwrap();
        dir
    }

    fn work_path(root: &TempDir) -> PathBuf {
        root.path().join(".astral/tasks/register.jsonl")
    }

    fn create(root: &TempDir) -> work_records::Result<work_records::Mutation> {
        work_records::create(
            root.path(),
            "New task",
            &["Acceptance".into()],
            &["AST-001".into()],
        )
    }

    #[test]
    fn create_update_and_project_load_round_trip_with_manifest_selected_path() {
        let root = fixture();
        let original = work_records::read(root.path()).unwrap();
        assert_eq!(original.path, ".astral/tasks/register.jsonl");
        assert_eq!(
            original.digest,
            astral::hash(&fs::read(work_path(&root)).unwrap())
        );
        fs::set_permissions(work_path(&root), fs::Permissions::from_mode(0o640)).unwrap();
        let old_inode = fs::metadata(work_path(&root)).unwrap().ino();
        let created = create(&root).unwrap();
        let id = &created.record.item.id;
        assert!(id.starts_with("AST-"));
        assert_eq!(id.len(), 16);
        assert!(
            id[4..]
                .bytes()
                .all(|b| b"0123456789abcdefghjkmnpqrstvwxyz".contains(&b))
        );
        assert_eq!(created.file.records.len(), 2);
        assert_eq!(
            created
                .file
                .records
                .iter()
                .find(|r| r.item.id == "AST-001")
                .unwrap()
                .digest,
            original.records[0].digest
        );
        assert_ne!(fs::metadata(work_path(&root)).unwrap().ino(), old_inode);
        assert_eq!(
            fs::metadata(work_path(&root)).unwrap().mode() & 0o777,
            0o640
        );
        let loaded = Project::load(root.path()).unwrap();
        assert_eq!(
            loaded.work_ids().collect::<Vec<_>>(),
            created
                .file
                .records
                .iter()
                .map(|r| r.item.id.as_str())
                .collect::<Vec<_>>()
        );
        let updated = work_records::update(
            root.path(),
            id,
            WorkStatus::Complete,
            &created.record.digest,
        )
        .unwrap();
        assert!(matches!(updated.record.item.status, WorkStatus::Complete));
        assert_eq!(updated.record.item.depends_on, ["AST-001"]);
        let read = work_records::read(root.path()).unwrap();
        assert_eq!(
            serde_json::to_value(&read).unwrap(),
            serde_json::to_value(&updated.file).unwrap()
        );
        Project::load(root.path()).unwrap();
        assert_eq!(
            fs::read_dir(work_path(&root).parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        assert!(!root.path().join(".astral/work/items.jsonl").exists());
    }

    #[test]
    fn update_retains_metadata_and_rejects_stale_digests_without_writing() {
        let root = fixture();
        let before = work_records::read(root.path()).unwrap();
        let first = &before.records[0];
        let updated = work_records::update(
            root.path(),
            "AST-001",
            WorkStatus::InProgress,
            &first.digest,
        )
        .unwrap();
        let mut expected = serde_json::to_value(&first.item).unwrap();
        expected["status"] = json!("in_progress");
        assert_eq!(
            serde_json::to_value(&updated.record.item).unwrap(),
            expected
        );
        let bytes = fs::read(work_path(&root)).unwrap();
        let inode = fs::metadata(work_path(&root)).unwrap().ino();
        let error =
            work_records::update(root.path(), "AST-001", WorkStatus::Complete, &first.digest)
                .unwrap_err();
        assert_eq!(error.code, "STALE_WORK_ITEM");
        assert_eq!(error.ids, ["AST-001"]);
        assert_eq!(fs::read(work_path(&root)).unwrap(), bytes);
        assert_eq!(fs::metadata(work_path(&root)).unwrap().ino(), inode);
        assert_eq!(
            work_records::update(root.path(), "AST-001", WorkStatus::Complete, "bad")
                .unwrap_err()
                .code,
            "INVALID_DIGEST"
        );
        assert_eq!(
            work_records::update(root.path(), "missing", WorkStatus::Complete, &first.digest)
                .unwrap_err()
                .code,
            "MISSING_WORK_ITEM"
        );
        assert_eq!(
            fs::read_dir(work_path(&root).parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn an_external_formatting_change_is_stale_but_unrelated_edits_are_allowed() {
        let root = fixture();
        let before = work_records::read(root.path()).unwrap();
        let original = fs::read(work_path(&root)).unwrap();
        let mut extra = original.clone();
        extra.extend(jsonl(&[item("AST-002", &[])]));
        fs::write(work_path(&root), extra).unwrap();
        let changed = work_records::update(
            root.path(),
            "AST-001",
            WorkStatus::Blocked,
            &before.records[0].digest,
        )
        .unwrap();
        assert_eq!(changed.file.records.len(), 2);
        let current = fs::read(work_path(&root)).unwrap();
        let mut formatted = b"  ".to_vec();
        formatted.extend(&current);
        fs::write(work_path(&root), &formatted).unwrap();
        assert_eq!(
            work_records::update(
                root.path(),
                "AST-001",
                WorkStatus::Complete,
                &changed.record.digest
            )
            .unwrap_err()
            .code,
            "STALE_WORK_ITEM"
        );
        assert_eq!(fs::read(work_path(&root)).unwrap(), formatted);
    }

    #[test]
    fn invalid_creation_and_invalid_existing_project_preserve_original() {
        let root = fixture();
        let original = fs::read(work_path(&root)).unwrap();
        let inode = fs::metadata(work_path(&root)).unwrap().ino();
        for (title, acceptance, deps, code) in [
            ("", vec!["criterion".into()], vec![], "INVALID_FIELD"),
            ("title", vec![], vec![], "INVALID_FIELD"),
            (
                "title",
                vec!["criterion".into()],
                vec!["missing".into()],
                "MISSING_REFERENCE",
            ),
        ] {
            assert_eq!(
                work_records::create(root.path(), title, &acceptance, &deps)
                    .unwrap_err()
                    .code,
                code
            );
            assert_eq!(fs::read(work_path(&root)).unwrap(), original);
            assert_eq!(fs::metadata(work_path(&root)).unwrap().ino(), inode);
        }
        assert_eq!(
            work_records::create(
                root.path(),
                &"x".repeat(MAX_FILE_BYTES),
                &["criterion".into()],
                &[]
            )
            .unwrap_err()
            .code,
            "LIMIT_EXCEEDED"
        );
        // The candidate parses, but the declared project is invalid.
        fs::rename(
            root.path().join(".astral/core/RUN.md"),
            root.path().join(".astral/core/OLD-RUN.md"),
        )
        .unwrap();
        assert!(create(&root).is_err());
        assert_eq!(fs::read(work_path(&root)).unwrap(), original);
        assert_eq!(fs::metadata(work_path(&root)).unwrap().ino(), inode);
        assert_eq!(
            fs::read_dir(work_path(&root).parent().unwrap())
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn aggregate_project_entries_are_reserved_before_mutation() {
        let root = fixture();
        let mut existing = item("AST-001", &[]);
        // 4090 work entries fit by themselves even with the new record's 3.
        // Projection directory entries + provenance use the remaining budget.
        existing["acceptance"] = json!((0..4088).map(|n| n.to_string()).collect::<Vec<_>>());
        fs::write(work_path(&root), jsonl(&[existing])).unwrap();
        for index in 0..3 {
            put(
                root.path(),
                &format!(".astral/projections/context-{index}/projection.toml"),
                format!(
                    "schema_version=1\nid='context-{index}'\nkind='test'\nsubsystems=[]\nhandoff='handoff.md'\nnative_payload_in_repository=false\n[[sources]]\nid='source'\nkind='test'\nscope='test'\navailability='test'\n"
                ),
            );
            put(
                root.path(),
                &format!(".astral/projections/context-{index}/handoff.md"),
                "Inert fixture",
            );
        }
        Project::load(root.path()).unwrap();
        let original = fs::read(work_path(&root)).unwrap();
        assert_eq!(create(&root).unwrap_err().code, "LIMIT_EXCEEDED");
        assert_eq!(fs::read(work_path(&root)).unwrap(), original);
    }

    #[test]
    fn aggregate_project_bytes_are_reserved_before_mutation() {
        let root = fixture();
        // Fifteen independently declared near-limit documents consume most of
        // the 16 MiB project budget without violating any per-file limit.
        for index in 0..12 {
            put(
                root.path(),
                &format!(".astral/projections/context-{index}/projection.toml"),
                format!(
                    "schema_version=1\nid='context-{index}'\nkind='test'\nsubsystems=[]\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n"
                ),
            );
            put(
                root.path(),
                &format!(".astral/projections/context-{index}/handoff.md"),
                vec![b'x'; MAX_FILE_BYTES],
            );
        }
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            put(
                root.path(),
                &format!(".astral/core/{name}"),
                vec![b'x'; MAX_FILE_BYTES],
            );
        }
        put(
            root.path(),
            ".astral/projections/final/projection.toml",
            "schema_version=1\nid='final'\nkind='test'\nsubsystems=[]\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n",
        );
        put(
            root.path(),
            ".astral/projections/final/handoff.md",
            vec![b'x'; MAX_FILE_BYTES - 16_384],
        );
        Project::load(root.path()).unwrap();
        let original = fs::read(work_path(&root)).unwrap();
        assert_eq!(
            work_records::create(root.path(), &"x".repeat(32_768), &["Criterion".into()], &[])
                .unwrap_err()
                .code,
            "LIMIT_EXCEEDED"
        );
        assert_eq!(fs::read(work_path(&root)).unwrap(), original);
    }

    #[test]
    fn advisory_directory_lock_excludes_writers_but_read_is_available() {
        let root = fixture();
        let lock = fs::File::open(root.path().join(".astral")).unwrap();
        fs2::FileExt::try_lock_exclusive(&lock).unwrap();
        let original = fs::read(work_path(&root)).unwrap();
        let observed = work_records::read(root.path()).unwrap();
        assert_eq!(create(&root).unwrap_err().code, "WORK_LOCKED");
        assert_eq!(
            work_records::update(
                root.path(),
                "AST-001",
                WorkStatus::Complete,
                &observed.records[0].digest
            )
            .unwrap_err()
            .code,
            "WORK_LOCKED"
        );
        assert_eq!(fs::read(work_path(&root)).unwrap(), original);
        drop(lock);
        create(&root).unwrap();
    }

    #[test]
    fn manifest_rejects_traversal_absolute_and_nonportable_paths() {
        for path in [
            "../outside.jsonl",
            "/tmp/outside.jsonl",
            "tasks/../tasks/register.jsonl",
            "./tasks/register.jsonl",
            "tasks//register.jsonl",
            "tasks\\register.jsonl",
            "C:register.jsonl",
        ] {
            let root = fixture();
            let original = fs::read(work_path(&root)).unwrap();
            let manifest = root.path().join(".astral/project.toml");
            let text = fs::read_to_string(&manifest)
                .unwrap()
                .replace("tasks/register.jsonl", path);
            fs::write(manifest, text).unwrap();
            assert_eq!(
                work_records::read(root.path()).unwrap_err().code,
                "UNSAFE_PATH",
                "{path}"
            );
            assert_eq!(create(&root).unwrap_err().code, "UNSAFE_PATH", "{path}");
            assert_eq!(fs::read(work_path(&root)).unwrap(), original);
        }
    }

    #[test]
    fn symlinked_work_manifest_and_directory_are_rejected_without_touching_targets() {
        for relative in [
            ".astral/tasks/register.jsonl",
            ".astral/project.toml",
            ".astral/tasks",
            ".astral",
        ] {
            let root = fixture();
            let source = root.path().join(relative);
            let moved = root.path().join("moved");
            fs::rename(&source, &moved).unwrap();
            symlink(&moved, &source).unwrap();
            let bytes = fs::read(work_path(&root)).unwrap();
            assert_eq!(
                work_records::read(root.path()).unwrap_err().code,
                "UNSAFE_FILE"
            );
            assert_eq!(create(&root).unwrap_err().code, "UNSAFE_FILE");
            assert_eq!(fs::read(work_path(&root)).unwrap(), bytes);
            assert!(
                fs::symlink_metadata(&source)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
        }
    }

    #[test]
    fn hardlinks_and_nonregular_files_are_rejected() {
        for relative in [".astral/tasks/register.jsonl", ".astral/project.toml"] {
            let root = fixture();
            fs::hard_link(root.path().join(relative), root.path().join("alias")).unwrap();
            let original = fs::read(work_path(&root)).unwrap();
            assert_eq!(
                work_records::read(root.path()).unwrap_err().code,
                "UNSAFE_FILE"
            );
            assert_eq!(create(&root).unwrap_err().code, "UNSAFE_FILE");
            assert_eq!(fs::read(work_path(&root)).unwrap(), original);
        }
        let root = fixture();
        fs::rename(work_path(&root), root.path().join("original")).unwrap();
        fs::create_dir(work_path(&root)).unwrap();
        assert_eq!(create(&root).unwrap_err().code, "UNSAFE_FILE");
        assert!(work_path(&root).is_dir());
        let fifo_root = fixture();
        fs::rename(work_path(&fifo_root), fifo_root.path().join("original")).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let fifo = std::ffi::CString::new(work_path(&fifo_root).as_os_str().as_bytes()).unwrap();
        // SAFETY: terminated owned path; creates a FIFO solely in this fixture.
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(create(&fifo_root).unwrap_err().code, "UNSAFE_FILE");
    }

    #[test]
    fn oversized_or_invalid_file_is_preserved_and_not_repaired_implicitly() {
        for bytes in [
            vec![b'x'; MAX_FILE_BYTES + 1],
            b"invalid private record\n".to_vec(),
            vec![0xff],
        ] {
            let root = fixture();
            fs::write(work_path(&root), &bytes).unwrap();
            assert!(create(&root).is_err());
            assert_eq!(fs::read(work_path(&root)).unwrap(), bytes);
        }
    }
}
