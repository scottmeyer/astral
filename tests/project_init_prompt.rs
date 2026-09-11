#[cfg(unix)]
#[test]
fn embedded_initialization_templates_form_a_valid_empty_work_project() {
    use astral::project::Project;
    use std::collections::BTreeSet;
    use std::fs;

    let prompt = include_str!("../src/prompts/project_init_v1.md");
    let root = tempfile::tempdir().unwrap();
    let mut lines = prompt.lines();
    let mut template_path = None;
    let mut written = BTreeSet::new();
    while let Some(line) = lines.next() {
        if let Some(path) = line
            .strip_prefix("### `")
            .and_then(|line| line.strip_suffix('`'))
        {
            template_path = Some(path);
        } else if line == "```toml" {
            let path = template_path
                .take()
                .expect("TOML template has a target heading");
            assert!(matches!(
                path,
                ".astral/project.toml"
                    | ".astral/core/project-context/subsystem.toml"
                    | ".astral/projections/initial-project-context/projection.toml"
            ));
            let mut content = String::new();
            let mut closed = false;
            for line in lines.by_ref() {
                if line == "```" {
                    closed = true;
                    break;
                }
                content.push_str(line);
                content.push('\n');
            }
            assert!(closed, "unterminated template for {path}");
            assert!(written.insert(path), "duplicate template for {path}");
            let destination = root.path().join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, content).unwrap();
        }
    }
    assert_eq!(written.len(), 3);
    // Markdown is grounded by the launched agent; the schema test supplies
    // synthetic contents and verifies the exact template references/identities.
    for path in [
        ".astral/core/ARCHITECTURE.md",
        ".astral/core/RUN.md",
        ".astral/core/TEST.md",
        ".astral/core/project-context/README.md",
        ".astral/projections/initial-project-context/handoff.md",
    ] {
        fs::write(
            root.path().join(path),
            "Synthetic source; no execution claims.\n",
        )
        .unwrap();
    }
    let work = root.path().join(".astral/work/items.jsonl");
    fs::create_dir_all(work.parent().unwrap()).unwrap();
    fs::write(&work, []).unwrap();
    let project = Project::load(root.path()).unwrap();
    project.validate().unwrap();
    let inspection = project.inspect("project-context", None).unwrap();
    assert_eq!(inspection["project_id"], "project");
    assert_eq!(inspection["selection"]["kind"], "subsystem");
    assert_eq!(inspection["native_binding"]["state"], "UNBOUND");
    assert_eq!(fs::metadata(work).unwrap().len(), 0);
    let saved = project
        .inspect("projection:initial-project-context", None)
        .unwrap();
    assert_eq!(
        saved["metadata"]["projection"]["native_payload_in_repository"],
        false
    );
    project.fresh_context("project-context", None).unwrap();
    project
        .fresh_context("projection:initial-project-context", None)
        .unwrap();
}
