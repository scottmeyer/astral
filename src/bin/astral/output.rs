//! Human CLI presentation. Library values and machine output remain unchanged.
#[path = "output/context.rs"]
mod context;
#[path = "output/workflow.rs"]
mod workflow;

use astral::project::{Error, Limits, Result};
use serde_json::Value;
use std::path::Path;

const MAX_ROWS: usize = 100;
const MAX_TEXT_CHARS: usize = 1024;

/// Render known command results using an explicit allowlist of useful fields.
/// Unknown shapes fail visibly rather than accidentally printing private data.
#[cfg(test)]
pub fn render(value: &Value) -> Result<String> {
    render_in(value, Path::new("."))
}

pub fn render_in(value: &Value, root: &Path) -> Result<String> {
    let mut out = Output::in_root(root);
    match text(value, "operation") {
        "initialize" => context::initialize(&mut out, value),
        "lifecycle_check" => workflow::lifecycle(&mut out, value),
        "context_freshness" | "context_review" => context::freshness(&mut out, value),
        "context_knowledge" | "context_knowledge_show" => context::knowledge(&mut out, value),
        "recovery_inventory" => workflow::recovery_inventory(&mut out, value),
        "recover" => workflow::recovered(&mut out, value),
        "finish_plan" => workflow::finish(&mut out, value, false),
        "finish_apply" => workflow::finish_result(&mut out, value),
        "save" => workflow::save(&mut out, value),
        "hooks_apply" => workflow::hooks_applied(&mut out, value),
        _ if value.get("contexts").is_some() => context::list(&mut out, value),
        _ if text(value, "status") == "valid" && value.get("project_id").is_some() => {
            context::validate(&mut out, value)
        }
        _ if value.get("selection").is_some() && text(value, "mode") == "inspect" => {
            context::inspect(&mut out, value)
        }
        _ if value.get("records").is_some() && value.get("path").is_some() => {
            context::work_list(&mut out, value)
        }
        _ if value.get("item").is_some() && value.get("digest").is_some() => {
            context::work_record(&mut out, value)
        }
        _ if matches!(text(value, "outcome"), "merged" | "conflicts") => {
            context::work_merge(&mut out, value)
        }
        _ if value.get("classification").is_some() && value.get("binding").is_some() => {
            workflow::recovery_plan(&mut out, value)
        }
        _ if value.get("target").is_some() && value.get("plan_sha256").is_some() => {
            workflow::hooks_plan(&mut out, value)
        }
        _ if value.get("git").is_some() && value.get("codex").is_some() => {
            workflow::hooks_status(&mut out, value)
        }
        _ if value
            .as_object()
            .is_some_and(|v| v.len() == 1 && v.contains_key("id")) =>
        {
            out.line(safe(text(value, "id")));
        }
        _ => {
            return Err(Error {
                code: "HUMAN_OUTPUT_UNSUPPORTED",
                message: "result presentation unavailable; the command may already have applied changes. Inspect current state before another mutation; use --json for future invocations".into(),
            });
        }
    }
    out.finish()
}

pub fn error_in(error: &Error, root: &Path) -> String {
    let mut result = self::error(error);
    if root != Path::new(".") {
        let mut location = Output::default();
        if let Some(root) = root.to_str() {
            location.command(
                "Before running the suggested follow-ups, use this project directory",
                &["cd", "--", root],
            );
        } else {
            location.line("Follow-up commands must retain the original --root argument; its non-UTF-8 path cannot be shown as a shell command.");
        }
        result.push('\n');
        result.push_str(&location.finish().expect("bounded root hint"));
    }
    result
}

fn error(error: &Error) -> String {
    let mut result = format!("Error [{}]: {}", safe(error.code), safe(&error.message));
    let advice = match error.code {
        "WORK_NOT_FOUND" | "MISSING_WORK" => "Run astral work list to choose an existing work ID.",
        "UNKNOWN_CONTEXT" | "AMBIGUOUS_CONTEXT" | "INVALID_SELECTOR" => {
            "Run astral context list; use subsystem:NAME or projection:NAME when needed."
        }
        "WORKSPACE_LOCKED" | "WORKSPACE_BUSY" => "Wait for the current worker owner, then retry.",
        "NATIVE_PROXY_REQUIRED" => "Repeat the project command with explicit --proxy.",
        "STALE_PLAN"
        | "FINISH_STALE_PLAN"
        | "RECOVERY_STALE_PLAN"
        | "HOOK_STALE_PLAN"
        | "FRESHNESS_REVIEW_STALE" => {
            "Generate a fresh preview and review its new hash before applying it."
        }
        "WORKSPACE_STAGE_INCOMPLETE" | "WORKSPACE_CONTEXT_INCOMPLETE" => {
            "Run astral status and astral recover; preserve the retained worker until its outcome is known."
        }
        _ => "",
    };
    if !advice.is_empty() {
        result.push('\n');
        result.push_str(advice);
    }
    result
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}
fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}
fn yes(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}
fn scalar(value: &Value) -> String {
    match value {
        Value::String(s) => safe(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => if *b { "yes" } else { "no" }.into(),
        _ => "not recorded".into(),
    }
}
fn safe(value: &str) -> String {
    let mut out: String = value
        .chars()
        .take(MAX_TEXT_CHARS)
        .flat_map(char::escape_debug)
        .collect();
    if value.chars().nth(MAX_TEXT_CHARS).is_some() {
        out.push_str("… [truncated; use --json]");
    }
    out
}

#[derive(Default)]
struct Output {
    root: Option<String>,
    root_unrepresentable: bool,
    text: String,
    exceeded: bool,
}
impl Output {
    fn in_root(root: &Path) -> Self {
        Self {
            root: (root != Path::new("."))
                .then(|| root.to_str().map(str::to_owned))
                .flatten(),
            root_unrepresentable: root.to_str().is_none(),
            ..Self::default()
        }
    }
    fn line(&mut self, line: impl AsRef<str>) {
        let line = line.as_ref();
        if self.text.len().saturating_add(line.len()).saturating_add(1)
            > Limits::default().output_bytes
        {
            self.exceeded = true;
            return;
        }
        self.text.push_str(line);
        self.text.push('\n');
    }
    fn field(&mut self, label: &str, value: &Value) {
        if !value.is_null() {
            self.line(format!("{label}: {}", scalar(value)));
        }
    }
    fn strings(&mut self, label: &str, values: &[Value]) {
        if values.is_empty() {
            return;
        }
        self.line(format!("{label}:"));
        for item in values.iter().take(MAX_ROWS) {
            self.line(format!("  - {}", scalar(item)));
        }
        self.more(values.len());
    }
    fn issues(&mut self, values: &[Value]) {
        for issue in values.iter().take(MAX_ROWS) {
            if issue.is_string() {
                self.line(format!("  ! {}", scalar(issue)));
            } else {
                self.line(format!(
                    "  ! {}: {}",
                    safe(text(issue, "code")),
                    safe(text(issue, "message"))
                ));
                if !text(issue, "guidance").is_empty() {
                    self.line(format!("    {}", safe(text(issue, "guidance"))));
                }
            }
        }
        self.more(values.len());
    }
    fn more(&mut self, count: usize) {
        if count > MAX_ROWS {
            self.line(format!(
                "  … {} more entries; use --json for the full result.",
                count - MAX_ROWS
            ));
        }
    }
    fn command(&mut self, label: &str, args: &[&str]) {
        let needs_root = args.first() == Some(&"astral")
            && !args
                .iter()
                .any(|arg| *arg == "--root" || arg.starts_with("--root="));
        if needs_root && self.root_unrepresentable {
            self.line(format!("{label}: retain the original --root argument; its non-UTF-8 path cannot be shown as a shell command."));
            return;
        }
        let root = self.root.clone();
        if let Some(root) = root.as_deref().filter(|_| needs_root) {
            let mut rooted = vec!["astral", "--root", root];
            rooted.extend_from_slice(&args[1..]);
            self.literal_command(label, &rooted);
        } else {
            self.literal_command(label, args);
        }
    }
    fn example(&mut self, default: &str, label: &str, args: &[&str]) {
        if self.root.is_none() && !self.root_unrepresentable {
            self.line(default);
        } else {
            self.command(label, args);
        }
    }
    fn literal_command(&mut self, label: &str, args: &[&str]) {
        if args.iter().any(|arg| {
            arg.len() > 4096
                || arg.chars().any(|c| {
                    c.is_control()
                        || c.escape_debug().count() > 1 && c != '\'' && c != '"' && c != '\\'
                })
        }) {
            self.line(format!("{label}: command contains non-displayable or oversized arguments; inspect literal argv with --json."));
            return;
        }
        let words: Vec<_> = args
            .iter()
            .map(|arg| {
                if !arg.is_empty()
                    && arg
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_./:=-".contains(&b))
                {
                    (*arg).to_owned()
                } else {
                    format!("'{}'", arg.replace('\'', "'\\''"))
                }
            })
            .collect();
        self.line(format!("{label}: {}", words.join(" ")));
    }
    fn arguments(&mut self, label: &str, value: &Value) {
        if let Some(items) = value.as_array() {
            if let Some(args) = items.iter().map(Value::as_str).collect::<Option<Vec<_>>>() {
                if !args.is_empty() {
                    self.literal_command(label, &args);
                }
            }
        }
    }
    fn argv(&mut self, label: &str, value: &Value) {
        if let Some(items) = value.as_array() {
            if let Some(args) = items.iter().map(Value::as_str).collect::<Option<Vec<_>>>() {
                if !args.is_empty() {
                    self.command(label, &args);
                }
            }
        }
    }
    fn finish(mut self) -> Result<String> {
        if self.exceeded {
            return Err(Error {
                code: "LIMIT_EXCEEDED",
                message: "human output byte limit; use --json for structured results".into(),
            });
        }
        if self.text.ends_with('\n') {
            self.text.pop();
        }
        Ok(self.text)
    }
}

#[cfg(test)]
#[path = "output/tests.rs"]
mod tests;
