//! Compact completed MCP output before a harness records it in conversation history.
use crate::{archive::Archive, config::Config, tool_history};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    #[default]
    Repetitions,
    Preview,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub scope: String,
    pub tool: String,
    #[serde(default)]
    pub policy: Policy,
    pub result: Value,
}

#[derive(Default, Serialize)]
pub struct Report {
    pub policy: Policy,
    pub archived: bool,
    pub bytes_in: usize,
    pub bytes_out: usize,
    pub skipped: Option<&'static str>,
}

fn numbered(line: &str) -> Option<(u64, usize, &str)> {
    let width = line.bytes().take_while(u8::is_ascii_digit).count();
    if width == 0 || width > 12 || !matches!(line.as_bytes().get(width), Some(b' ' | b'\t')) {
        return None;
    }
    Some((line[..width].parse().ok()?, width, &line[width..]))
}

/// Fold exact repeated lines or consecutive decimal labels with identical text.
/// Never infer timestamp, severity or relevance; preserve all other lines.
fn repetitions(text: &str) -> String {
    let lines: Vec<_> = text.split_inclusive('\n').collect();
    let mut output = String::new();
    let mut i = 0;
    while i < lines.len() {
        let first = lines[i];
        let mut end = i + 1;
        if first.len() >= 32 {
            if let Some((start, width, rest)) = numbered(first) {
                while end < lines.len()
                    && numbered(lines[end]).is_some_and(|(n, w, r)| {
                        w == width && r == rest && start.checked_add((end - i) as u64) == Some(n)
                    })
                {
                    end += 1;
                }
            }
            if end == i + 1 {
                while end < lines.len() && lines[end] == first {
                    end += 1;
                }
            }
        }
        if end - i >= 4 {
            output.push_str(first);
            output.push_str(&format!(
                "[astral repeated sequence: {} lines total; {} interior lines omitted]\n",
                end - i,
                end - i - 2
            ));
            output.push_str(lines[end - 1]);
            i = end;
        } else {
            output.push_str(first);
            i += 1;
        }
    }
    output
}

/// Rich results, failures and already compacted output retain their exact values.
pub fn text(result: &Value) -> Option<String> {
    let object = result.as_object()?;
    if object.keys().any(|k| k != "content" && k != "isError")
        || object.get("isError").is_some_and(|v| v != false)
    {
        return None;
    }
    let blocks = result["content"].as_array()?;
    if blocks.is_empty() {
        return None;
    }
    let mut texts = Vec::new();
    for block in blocks {
        let object = block.as_object()?;
        if object.keys().any(|k| k != "type" && k != "text") || block["type"] != "text" {
            return None;
        }
        let text = block["text"].as_str()?;
        if tool_history::explicit_failure(text) || text.contains("[astral archived tool output;") {
            return None;
        }
        texts.push(text);
    }
    Some(texts.join("\n"))
}

pub fn compact(request: &mut Request, config: &Config, archive: &Archive) -> Report {
    let original = serde_json::to_string(&request.result).expect("serializable MCP result");
    let mut report = Report {
        policy: request.policy,
        bytes_in: original.len(),
        bytes_out: original.len(),
        ..Report::default()
    };
    let Some(display) = text(&request.result) else {
        report.skipped = Some("protected_result");
        return report;
    };
    if display.len() < config.tool_result_bytes {
        report.skipped = Some("below_threshold");
        return report;
    }
    let (view, description) = match request.policy {
        Policy::Repetitions => (
            repetitions(&display),
            "Consecutive repeated lines reduced; distinct lines retained. Sequence notices give the original count and retain the first and last lines. This is not the complete original result.",
        ),
        Policy::Preview => (
            tool_history::preview(&display, config.tool_preview_bytes),
            "Incomplete head/tail preview of the original MCP result.",
        ),
    };
    if view.len().saturating_add(1024) >= display.len() {
        report.skipped = Some("no_savings");
        return report;
    }
    let handle = match archive.save(&request.scope, "json", &original) {
        Ok(handle) => handle,
        Err(_) => {
            report.skipped = Some("archive_unavailable");
            return report;
        }
    };
    let label = serde_json::to_string(&request.tool).unwrap();
    let marker = format!(
        "[astral archived tool output; tool={label}; original_bytes={}; format=json; handle={handle}]\n{description} Use astral_search(handle=\"{handle}\", query=\"literal text\") to find byte offsets, then astral_recall(handle=\"{handle}\", offset=..., limit=...) for exact data. Maximum page: 16384 bytes; continue with next_offset. Retrieval does not rerun the tool.\n{view}",
        original.len()
    );
    let mut replacement = request.result.clone();
    replacement["content"] = json!([{"type":"text", "text":marker}]);
    let after = serde_json::to_vec(&replacement).unwrap().len();
    if after >= original.len() {
        report.skipped = Some("no_savings");
        return report;
    }
    request.result = replacement;
    report.archived = true;
    report.bytes_out = after;
    report
}
