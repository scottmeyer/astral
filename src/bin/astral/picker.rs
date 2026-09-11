//! Interactive selection only. All mutations stay in the existing launcher.
#[path = "picker/model.rs"]
mod model;
#[path = "picker/terminal.rs"]
mod terminal;

use astral::launcher::ProjectArguments;
use astral::project::{Error, Result};
use model::{Catalog, Review};
use terminal::{Choice, Selection, Terminal};

pub(super) use terminal::available;

fn error(code: &'static str, message: impl Into<String>) -> Error {
    Error {
        code,
        message: message.into(),
    }
}

fn terminal_error(error: std::io::Error) -> Error {
    self::error(
        "PICKER_TERMINAL",
        format!("cannot use the context picker: {error}; use astral context list --plain"),
    )
}

pub(super) async fn run(request: ProjectArguments) -> Result<Option<serde_json::Value>> {
    if !available() {
        return Err(error(
            "PICKER_REQUIRES_TTY",
            "the picker needs an interactive terminal; use astral context list --plain or astral project CONTEXT",
        ));
    }
    let catalog = Catalog::load(&request.root)?;
    let mut terminal = Terminal::open().map_err(terminal_error)?;
    let mut context_index = catalog.context_index(&request.name)?;
    'contexts: loop {
        match terminal
            .select(
                "Choose a context",
                &catalog.context_choices(),
                context_index,
            )
            .map_err(terminal_error)?
        {
            Selection::Chosen(index) => context_index = index,
            Selection::Back | Selection::Cancel => return Ok(None),
        }
        let mut work_index = catalog.work_index(request.work.as_deref())?;
        loop {
            match terminal
                .select("Choose a work item", &catalog.work_choices(), work_index)
                .map_err(terminal_error)?
            {
                Selection::Chosen(index) => work_index = index,
                Selection::Back => continue 'contexts,
                Selection::Cancel => return Ok(None),
            }
            let mut selected = request.clone();
            selected.name = catalog.contexts[context_index].selector.clone();
            selected.work = work_index
                .checked_sub(1)
                .map(|index| catalog.work[index].item.id.clone());
            let review = match Review::load(selected.clone()).await {
                Ok(review) => review,
                Err(error) => {
                    let detail = vec![
                        format!("{}: {}", error.code, error.message),
                        "Resolve this issue or choose another work item.".into(),
                    ];
                    if matches!(
                        terminal
                            .select(
                                "Cannot launch this selection",
                                &[Choice {
                                    label: "Back".into(),
                                    searchable: "back".into(),
                                    detail
                                }],
                                0
                            )
                            .map_err(terminal_error)?,
                        Selection::Cancel
                    ) {
                        return Ok(None);
                    }
                    continue;
                }
            };
            let choices = vec![
                review.choice(),
                Choice {
                    label: "Back".into(),
                    detail: vec!["Return to work selection.".into()],
                    searchable: "back".into(),
                },
            ];
            match terminal
                .review("Review launch", &choices, 0)
                .map_err(terminal_error)?
            {
                Selection::Chosen(0) => {
                    // Restore the terminal before any process, proxy or worktree is started.
                    drop(terminal);
                    let current = Review::load(selected).await?;
                    if current.stamp != review.stamp {
                        return Err(error(
                            "PICKER_CHANGED",
                            "context, Git state or worker binding changed during review; reopen the picker to review the current selection",
                        ));
                    }
                    return super::run_project(review.request).await;
                }
                Selection::Cancel => return Ok(None),
                Selection::Back | Selection::Chosen(_) => {}
            }
        }
    }
}
