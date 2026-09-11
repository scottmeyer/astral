use super::*;
use astral::project::{Result, freshness::review};
use std::io::{IsTerminal, Write};

fn io_error(_: std::io::Error) -> Error {
    Error {
        code: "CONTEXT_CONFIRMATION_IO",
        message:
            "context review terminal input/output failed; inspect context freshness before retrying"
                .into(),
    }
}

pub(super) fn run(
    root: &Path,
    name: &str,
    apply: Option<&str>,
    yes: bool,
    dry_run: bool,
    json: bool,
) -> Result<Option<Value>> {
    if let Some(expected) = apply {
        return review(root, name, Some(expected)).map(Some);
    }
    let plan = review(root, name, None)?;
    let interactive = !json && std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if dry_run || (!yes && !interactive) {
        return Ok(Some(plan));
    }
    if !yes {
        let mut display = plan.clone();
        display["interactive_confirmation"] = json!(true);
        let mut output = std::io::stderr().lock();
        writeln!(output, "{}", super::output::render_in(&display, root)?).map_err(io_error)?;
        if !super::confirmation::ask(
            &mut std::io::stdin().lock(),
            &mut output,
            "Record this review acknowledgement?",
        )
        .map_err(io_error)?
        {
            writeln!(output, "Cancelled; no review recorded.").map_err(io_error)?;
            return Ok(None);
        }
    }
    // Reuse the exact preview; edits during confirmation fail the same apply checks.
    review(root, name, plan["plan_sha256"].as_str()).map(Some)
}
