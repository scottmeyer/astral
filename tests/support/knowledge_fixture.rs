use astral::project::{Project, freshness};
use serde_json::Value;
use std::{fs, path::Path, process::Command};

pub const MANIFEST: &str = ".astral/core/web/subsystem.toml";
pub const DOCUMENT: &str = ".astral/core/web/README.md";
pub const REGIONS: &str = "Intro\n<!-- astral:begin sessions -->\nSession claim.\n<!-- astral:end sessions -->\n\n<!-- astral:begin routing -->\nRouting claim.\n<!-- astral:end routing -->\n";
const HEADER: &str = "# Kept comment\nschema_version=1\nid='web'\npurpose='web knowledge'\nreadme='README.md'\nrules=[]\ndecisions=[]\nwork_items=[]\nprojection='seed'\ndepends_on=[]\n";

pub struct Fixture(pub tempfile::TempDir);
impl Fixture {
    pub fn new() -> Self {
        let f = Self(tempfile::tempdir().unwrap());
        f.put(".astral/project.toml", "schema_version=1\nid='fixture'\nname='Fixture'\ndescription='Knowledge'\ncore='core'\nprojections='projections'\nwork_items='work/items.jsonl'\n[identity]\nscope='repository'\nruntime_bindings='private'\n[subsystems]\nweb='core/web'\n");
        for name in ["ARCHITECTURE.md", "RUN.md", "TEST.md"] {
            f.put(
                &format!(".astral/core/{name}"),
                "Historical verification.\n",
            );
        }
        f.put(".astral/work/items.jsonl", "");
        f.put(".astral/projections/seed/projection.toml", "schema_version=1\nid='seed'\nkind='fresh-context'\nsubsystems=['web']\nhandoff='handoff.md'\nnative_payload_in_repository=false\nsources=[]\n");
        f.put(".astral/projections/seed/handoff.md", "Historical handoff");
        f.put(DOCUMENT, REGIONS);
        f.put("src/session.rs", "SECRET_SESSION_CODE");
        f.put("src/router.rs", "SECRET_ROUTER_CODE");
        f.define(&format!(
            "{}{}",
            Self::entry("sessions", Some("sessions"), "src/session.rs"),
            Self::entry("routing", Some("routing"), "src/router.rs")
        ));
        f
    }
    pub fn root(&self) -> &Path {
        self.0.path()
    }
    pub fn put(&self, path: &str, text: impl AsRef<[u8]>) {
        let path = self.root().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    pub fn define(&self, entries: &str) {
        self.put(MANIFEST, format!("{HEADER}{entries}"));
    }
    pub fn entry(id: &str, region: Option<&str>, input: &str) -> String {
        let region = region
            .map(|r| format!("region='{r}'\n"))
            .unwrap_or_default();
        format!(
            "\n[[knowledge]]\nid='{id}'\nkind='decision'\ntitle='{id} decision'\ndocument='README.md'\n{region}inputs=['{input}']\n"
        )
    }
    pub fn project(&self) -> Project {
        Project::load(self.root()).unwrap()
    }
    pub fn observation(&self, id: &str) -> Value {
        self.project()
            .inspect_freshness(&format!("knowledge:web/{id}"))
            .unwrap()["freshness"][0]
            .clone()
    }
    pub fn review(&self, selector: &str) {
        let plan = freshness::review(self.root(), selector, None).unwrap();
        let result =
            freshness::review(self.root(), selector, plan["plan_sha256"].as_str()).unwrap();
        assert_eq!(result["applied"], true);
    }
    pub fn git(&self, args: &[&str]) {
        let mut cmd = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                cmd.env_remove(key);
            }
        }
        let out = cmd
            .current_dir(self.root())
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_COUNT", "3")
            .env("GIT_CONFIG_KEY_0", "commit.gpgsign")
            .env("GIT_CONFIG_VALUE_0", "false")
            .env("GIT_CONFIG_KEY_1", "maintenance.auto")
            .env("GIT_CONFIG_VALUE_1", "false")
            .env("GIT_CONFIG_KEY_2", "gc.auto")
            .env("GIT_CONFIG_VALUE_2", "0")
            .env("GIT_AUTHOR_NAME", "Knowledge Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
            .env("GIT_COMMITTER_NAME", "Knowledge Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
