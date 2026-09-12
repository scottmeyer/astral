use super::*;

fn reference(value: &Value) -> String {
    if let Some(entry) = value["knowledge"].as_str() {
        format!("knowledge:{}/{entry}", text(value, "subsystem"))
    } else {
        format!("subsystem:{}", text(value, "subsystem"))
    }
}

pub(in super::super) fn freshness_rows(out: &mut Output, value: &Value) {
    let rows = array(value, "freshness");
    for row in rows.iter().take(MAX_ROWS) {
        let state = match text(row, "state") {
            "unchanged" => "unchanged since review",
            "unreviewed" => "no review baseline",
            "needs_review" => "inputs or knowledge changed; review needed",
            _ => "inputs unavailable; review cannot be recorded",
        };
        out.line(format!("  {}: {state}", safe(&reference(row))));
    }
    out.more(rows.len());
}

pub(in super::super) fn freshness(out: &mut Output, value: &Value) {
    out.line(if yes(value, "applied") {
        "Context review recorded."
    } else if text(value, "operation") == "context_review" {
        "Context review preview"
    } else {
        "Context freshness"
    });
    let rows = array(value, "freshness");
    if rows.is_empty() {
        out.line("No freshness inputs declared for this selection. Add [freshness] with inputs = [\"path/to/code\"] to a subsystem.toml to opt in.");
        return;
    }
    freshness_rows(out, value);
    for row in rows.iter().take(MAX_ROWS) {
        let document = &row["document"];
        if !document.is_null() {
            out.field("    Document", &document["path"]);
            out.field("    Region", &document["region"]);
            out.field("    Problem", &document["error_code"]);
        }
        for input in array(row, "inputs").iter().take(MAX_ROWS) {
            out.line(format!(
                "    {}{}",
                safe(text(input, "path")),
                if input["error_code"].is_null() {
                    String::new()
                } else {
                    format!(" — {}", safe(text(input, "error_code")))
                }
            ));
        }
        if text(value, "operation") != "context_review" {
            out.command(
                "Review this reference",
                &["astral", "context", "review", &reference(row)],
            );
        }
    }
    out.line("Equal fingerprints mean unchanged since review; they do not verify documentation or tests.");
    if text(value, "operation") == "context_review" {
        out.field("Manifest", &value["manifest"]);
        out.field("Scope", &value["scope"]);
        if yes(value, "applied") {
            out.line("Review the diff, then stage the manifest with the corresponding code and documents.");
        } else if !yes(value, "interactive_confirmation") {
            out.line("Review the declared code and knowledge covered by this reference, then acknowledge this preview:");
            out.command(
                "Record review",
                &[
                    "astral",
                    "context",
                    "review",
                    text(value, "reference"),
                    "--apply",
                    text(value, "plan_sha256"),
                ],
            );
            out.line("In a terminal, the same review command offers confirmation. Use --yes to acknowledge current inputs without prompting, or --dry-run to keep a preview.");
        }
    }
}
