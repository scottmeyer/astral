//! Bounded read-only catalog and launch review. No runtime authority is imported.
use super::{error, terminal::Choice};
use astral::launcher::{ProjectArguments, Route};
use astral::project::{Project, Result, WorkStatus};
use astral::work_records::{self, Record};
use astral::workspace::{OwnershipObservation, WorktreeBinding};
use serde_json::{Value, json};
use std::path::Path;

pub(super) struct Context {
    pub selector: String,
    purpose: String,
    native: bool,
}

pub(super) struct Catalog {
    pub contexts: Vec<Context>,
    pub work: Vec<Record>,
}

impl Catalog {
    pub fn load(root: &Path) -> Result<Self> {
        let project = Project::load(root)?;
        let list = project.list()?;
        let mut contexts = Vec::new();
        for row in list["contexts"].as_array().expect("context list") {
            let selector = row["selector"]
                .as_str()
                .expect("context selector")
                .to_owned();
            let inspected = project.inspect(&selector, None)?;
            let purpose = inspected["metadata"]["subsystems"]
                [row["id"].as_str().expect("context id")]["purpose"]
                .as_str()
                .unwrap_or(if row["kind"] == "projection" {
                    "Saved project context"
                } else {
                    "Project subsystem"
                })
                .chars()
                .take(512)
                .collect();
            let native = !project.native_artifacts(&selector, None)?.is_empty();
            contexts.push(Context {
                selector,
                purpose,
                native,
            });
        }
        if contexts.is_empty() {
            return Err(error(
                "PICKER_EMPTY",
                "no contexts are available; define a subsystem or projection in .astral",
            ));
        }
        let mut work = work_records::read(root)
            .map_err(|e| error(e.code, e.message))?
            .records;
        work.sort_by(|left, right| {
            (rank(&left.item.status), &left.item.id)
                .cmp(&(rank(&right.item.status), &right.item.id))
        });
        Ok(Self { contexts, work })
    }

    pub fn context_index(&self, selector: &str) -> Result<usize> {
        if let Some(index) = self
            .contexts
            .iter()
            .position(|context| context.selector == selector)
        {
            return Ok(index);
        }
        let matches: Vec<_> = self
            .contexts
            .iter()
            .enumerate()
            .filter(|(_, c)| {
                c.selector
                    .split_once(':')
                    .is_some_and(|(_, id)| id == selector)
            })
            .collect();
        match matches.as_slice() {
            [(index, _)] => Ok(*index),
            [] if selector == astral::launcher::DEFAULT_CONTEXT => Ok(0),
            [] => Err(error(
                "CONTEXT_NOT_FOUND",
                "requested context is not in the picker catalog",
            )),
            _ => Err(error(
                "AMBIGUOUS_CONTEXT",
                "use subsystem:NAME or projection:NAME to preselect this context",
            )),
        }
    }

    pub fn work_index(&self, work: Option<&str>) -> Result<usize> {
        work.map_or(Ok(0), |id| {
            self.work
                .iter()
                .position(|r| r.item.id == id)
                .map(|i| i + 1)
                .ok_or_else(|| {
                    error(
                        "WORK_NOT_FOUND",
                        "requested work ID is not in this checkout",
                    )
                })
        })
    }

    pub fn context_choices(&self) -> Vec<Choice> {
        self.contexts
            .iter()
            .map(|c| Choice {
                label: c.selector.clone(),
                searchable: format!("{} {}", c.selector, c.purpose),
                detail: vec![
                    c.purpose.clone(),
                    if c.native {
                        "Native checkpoint; launch requires --proxy."
                    } else {
                        "Start from the selected project documents."
                    }
                    .into(),
                ],
            })
            .collect()
    }

    pub fn work_choices(&self) -> Vec<Choice> {
        let mut choices = vec![Choice {
            label: "Continue without a work item".into(),
            searchable: "none without work item".into(),
            detail: vec!["Launch in the current checkout without a bound worker.".into()],
        }];
        choices.extend(self.work.iter().map(|record| {
            let item = &record.item;
            let mut detail = vec![
                format!("Work status: {}", status(&item.status)),
                "Existing workers use their recorded context; the next screen shows the binding."
                    .into(),
            ];
            detail.extend(
                item.acceptance
                    .iter()
                    .take(3)
                    .map(|s| s.chars().take(512).collect()),
            );
            Choice {
                label: format!(
                    "{}  [{}]  {}",
                    item.id,
                    status(&item.status),
                    item.title.chars().take(256).collect::<String>()
                ),
                searchable: format!(
                    "{} {} {}",
                    item.id,
                    item.title.chars().take(3000).collect::<String>(),
                    status(&item.status)
                ),
                detail,
            }
        }));
        choices
    }
}

fn rank(status: &WorkStatus) -> u8 {
    match status {
        WorkStatus::InProgress => 0,
        WorkStatus::Open => 1,
        WorkStatus::Blocked => 2,
        WorkStatus::Complete => 3,
    }
}
fn status(status: &WorkStatus) -> &'static str {
    match status {
        WorkStatus::InProgress => "in progress",
        WorkStatus::Open => "open",
        WorkStatus::Blocked => "blocked",
        WorkStatus::Complete => "complete",
    }
}

pub(super) struct Review {
    pub request: ProjectArguments,
    pub stamp: String,
    detail: Vec<String>,
    action: String,
}

impl Review {
    pub async fn load(mut request: ProjectArguments) -> Result<Self> {
        request.root = request
            .root
            .canonicalize()
            .map_err(|_| error("INVALID_ROOT", "project root is unavailable"))?;
        let source = Project::load(&request.root)?;
        let original_selector = request.name.clone();
        let mut effective_root = request.root.clone();
        let mut execution_root = request.root.clone();
        let mut binding = Value::Null;
        let mut worker_context = Value::Null;
        let mut requires_proxy = false;
        let mut resume = false;
        let mut work_detail = "Work: none (current checkout)".to_owned();
        let mut branch = String::new();
        if let Some(work) = &request.work {
            let report = astral::status::collect(&request.root, Some(work), 0, 1)?;
            if let Some(diagnostic) = report.diagnostics.first() {
                return Err(error(
                    "PICKER_WORK_UNAVAILABLE",
                    format!("{}: {}", diagnostic.code, diagnostic.message),
                ));
            }
            let worker = report
                .workers
                .first()
                .ok_or_else(|| error("WORK_NOT_FOUND", "work item is unavailable"))?;
            if !worker.diagnostics.is_empty() || worker.state == "owned" {
                return Err(error(
                    "PICKER_WORK_UNAVAILABLE",
                    format!(
                        "{} needs attention or is already running; run astral doctor --work {}",
                        work, work
                    ),
                ));
            }
            work_detail = format!("Work: {} — {}", work, worker.title);
            worker_context = json!(worker.context);
            requires_proxy = worker
                .context
                .as_ref()
                .is_some_and(|context| context.requires_proxy);
            let observed = worker.binding.as_ref().ok_or_else(|| {
                error(
                    "PICKER_WORK_UNAVAILABLE",
                    "worker binding could not be observed",
                )
            })?;
            if observed.plan.is_some() && observed.ownership != OwnershipObservation::Available {
                return Err(error(
                    "PICKER_WORK_UNAVAILABLE",
                    "worker ownership is unavailable; run astral doctor before resuming",
                ));
            }
            binding = json!(observed);
            if let Some(plan) = &observed.plan {
                request.name = observed.selector.clone().ok_or_else(|| {
                    error("PICKER_WORK_UNAVAILABLE", "worker has no recorded selector")
                })?;
                effective_root = plan.root.clone();
                execution_root = plan.root.clone();
                branch = plan.branch.clone();
                resume = plan.worker_metadata.thread_id.is_some();
            } else {
                let records =
                    work_records::read(&request.root).map_err(|e| error(e.code, e.message))?;
                astral::git_context::require_committed_context(&request.root, &records.path)
                    .await?;
                let plan = WorktreeBinding::inspect(
                    &request.root,
                    source.project_id(),
                    &request.name,
                    work,
                )?;
                execution_root = plan.root.clone();
                branch = plan.branch.clone();
                binding = json!({ "observation": observed, "planned": plan });
                work_detail.push_str(" (new worker)");
            }
        }
        let effective = Project::load(&effective_root)?;
        astral::codex::StagingOptions::from_args(&execution_root, &request.codex_args)?;
        let selected = effective.launch_context(&request.name, request.work.as_deref())?;
        requires_proxy |= selected.native.is_some();
        if request.route == Route::Proxy && !requires_proxy && request.work.is_none() {
            return Err(error(
                "PROXY_REQUIRES_NATIVE",
                "this context starts from documents; omit --proxy or choose a native checkpoint",
            ));
        }
        let source_git = git_state(&request.root).await?;
        let effective_git = if effective_root == request.root {
            source_git.clone()
        } else {
            git_state(&effective_root).await?
        };
        if branch.is_empty() {
            branch = effective_git["branch"]
                .as_str()
                .unwrap_or("detached HEAD")
                .to_owned();
        }
        let worktree = binding
            .get("planned")
            .and_then(|p| p.get("root"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| effective_root.display().to_string());
        let mut detail = vec![
            format!("Context: {}", request.name),
            work_detail,
            format!("Branch: {branch}"),
            format!("Worktree: {worktree}"),
        ];
        if request.name != original_selector {
            detail.push(format!(
                "Using this worker's recorded context (selected: {original_selector})."
            ));
        }
        if requires_proxy {
            request.route = Route::Proxy;
        }
        detail.push(
            if request.route == Route::Proxy {
                "Route: --proxy (explicitly enabled by choosing this launch action)"
            } else {
                "Route: direct"
            }
            .into(),
        );
        if !request.codex_args.is_empty() {
            detail.push(
                format!("Codex arguments: {:?}", request.codex_args)
                    .chars()
                    .take(1024)
                    .collect(),
            );
        }
        detail.push("Enter launches Codex. Esc returns to work selection.".into());
        let action = format!(
            "{}{}",
            if resume { "Resume" } else { "Launch" },
            if request.route == Route::Proxy {
                " with --proxy"
            } else {
                ""
            }
        );
        let stamp = astral::fingerprint(
            &json!({ "sources": source.observed_sources().collect::<Vec<_>>(), "selection": selected.current_context.selection_digest, "binding": binding, "worker_context": worker_context, "source_git": source_git, "effective_git": effective_git,
                "request": { "root": request.root, "name": request.name, "work": request.work, "route": request.route,
                    "args": request.codex_args.iter().map(|arg| arg.as_encoded_bytes()).collect::<Vec<_>>() }
            }),
        );
        Ok(Self {
            request,
            stamp,
            detail,
            action,
        })
    }

    pub fn choice(&self) -> Choice {
        Choice {
            label: self.action.clone(),
            searchable: self.action.clone(),
            detail: self.detail.clone(),
        }
    }
}

async fn git_state(root: &Path) -> Result<Value> {
    let head = astral::git_context::read(root, &["rev-parse", "--verify", "HEAD"]).await?;
    let branch = astral::git_context::read(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    let status = astral::git_context::read(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )
    .await?;
    Ok(
        json!({ "head": String::from_utf8_lossy(&head).trim(), "branch": String::from_utf8_lossy(&branch).trim(), "status_digest": astral::fingerprint(&json!(status)) }),
    )
}
