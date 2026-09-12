use super::*;

pub(in super::super) fn knowledge(out: &mut Output, value: &Value) {
    if text(value, "operation") == "context_knowledge_show" {
        out.line(format!(
            "{} — {}",
            safe(text(value, "reference")),
            safe(text(value, "title"))
        ));
        freshness_rows(out, value);
        out.line("");
        let lines: Vec<_> = text(value, "text").lines().collect();
        for line in lines.iter().take(MAX_ROWS) {
            out.line(safe(line));
        }
        out.more(lines.len());
        out.line("Project knowledge; historical verification remains historical. Use --json for complete bounded text and source metadata.");
    } else {
        out.line("Named knowledge");
        let rows = array(value, "knowledge");
        if rows.is_empty() {
            out.line("No named entries in this selection. Declare [[knowledge]] in a subsystem.toml to add one.");
        }
        for row in rows.iter().take(MAX_ROWS) {
            out.line(format!(
                "  {} — {} ({})",
                safe(text(row, "reference")),
                safe(text(row, "title")),
                safe(text(&row["observation"], "state"))
            ));
        }
        out.more(rows.len());
        if let Some(first) = rows.first() {
            out.command(
                "Recall an entry",
                &["astral", "context", "show", text(first, "reference")],
            );
        }
    }
}
