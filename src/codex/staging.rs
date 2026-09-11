//! Fresh document, native import and bound-worker staging policies.

use super::error;
use super::options::StagingOptions;
use super::rpc::{Rpc, initialize_rpc, verify_native_version};
use crate::native_bundle::NativeBundle;
use crate::project::Result;
use serde::Serialize;
use serde_json::{Value, json};
use std::ffi::{OsStr, OsString};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct StagedThread {
    pub thread_id: String,
    pub model: String,
    pub model_provider: String,
}

pub(crate) enum StagingProgress {
    Starting,
    Injecting,
    Injected(StagedThread),
}

fn valid_thread_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn thread_id(value: &Value) -> Option<&str> {
    value
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .filter(|id| valid_thread_id(id))
}

fn durable_thread_matches(value: &Value, id: &str, expected_id: Option<&str>) -> bool {
    expected_id.is_none_or(|expected| expected == id)
        && value.pointer("/thread/ephemeral").and_then(Value::as_bool) == Some(false)
}

fn workspace_matches(cwd: &str, root: &Path) -> bool {
    Path::new(cwd).canonicalize().ok().as_deref() == Some(root)
}

fn context_message(context: &str) -> Value {
    json!({"type":"message","role":"user","content":[{"type":"input_text","text":context}]})
}

async fn inject_context(rpc: &mut Rpc, thread: &str, context: &str) -> Result<()> {
    rpc.request(
        "thread/inject_items",
        json!({"threadId":thread,"items":[context_message(context)]}),
    )
    .await?;
    Ok(())
}

/// Call only after all no-thread preflights. The hook stays immediately before
/// the first start request so an unknown outcome retains destination ownership.
async fn start_or_resume<F: FnMut(StagingProgress) -> Result<()>>(
    rpc: &mut Rpc,
    mut params: Value,
    resume: Option<&str>,
    progress: &mut F,
) -> Result<Value> {
    let method = if let Some(id) = resume {
        params["threadId"] = json!(id);
        params["excludeTurns"] = json!(true);
        "thread/resume"
    } else {
        progress(StagingProgress::Starting)?;
        "thread/start"
    };
    rpc.request(method, params).await
}

pub(crate) fn proxy_args(options: &StagingOptions, proxy: Option<&str>) -> Result<Vec<OsString>> {
    let Some(url) = proxy else {
        return Ok(options.server_args.clone());
    };
    crate::launcher::ProxyBinding::loopback(url)?;
    // Owned routing precedes caller overrides, as it does in the child command.
    // Verify the effective configuration before importing context.
    let mut args: Vec<OsString> = vec![
        "app-server".into(),
        "--listen".into(),
        "stdio://".into(),
        "-c".into(),
        format!("openai_base_url={url:?}").into(),
        "-c".into(),
        "features.enable_request_compression=false".into(),
    ];
    args.extend(options.server_args.iter().skip(3).cloned());
    Ok(args)
}

async fn effective_proxy_matches(
    rpc: &mut Rpc,
    root: &Path,
    url: &str,
    provider: &str,
) -> Result<bool> {
    let value = rpc
        .request("config/read", json!({"cwd":root,"includeLayers":false}))
        .await?;
    let config = value
        .get("config")
        .ok_or_else(|| error("CODEX_PROTOCOL", "missing effective configuration"))?;
    Ok(
        config.get("openai_base_url").and_then(Value::as_str) == Some(url)
            && config
                .pointer("/features/enable_request_compression")
                .and_then(Value::as_bool)
                == Some(false)
            && config
                .pointer("/model_providers/openai")
                .is_none_or(Value::is_null)
            && config
                .get("model_provider")
                .and_then(Value::as_str)
                .is_none_or(|v| v == provider),
    )
}

pub(crate) async fn verify_proxy(rpc: &mut Rpc, root: &Path, proxy: Option<&str>) -> Result<()> {
    if let Some(url) = proxy {
        if !effective_proxy_matches(rpc, root, url, "openai").await? {
            return Err(error(
                "NATIVE_ROUTE_CONFLICT",
                "effective configuration differs from the owned proxy route",
            ));
        }
    }
    Ok(())
}

/// Create a new thread and persist exactly one user-context message. No session
/// file, credential or native artifact is read or patched by this adapter.
pub async fn stage_fresh(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    context: &str,
) -> Result<String> {
    let mut rpc = Rpc::spawn(executable, root, &options.server_args)?;
    let result = async {
        initialize_rpc(&mut rpc).await?;
        let started = rpc.request("thread/start", options.thread_params).await?;
        let id = thread_id(&started)
            .ok_or_else(|| error("CODEX_PROTOCOL", "invalid staged thread ID"))?;
        if !durable_thread_matches(&started, id, None) {
            return Err(error(
                "CODEX_PROTOCOL",
                "staged thread must report durable storage",
            ));
        }
        let cwd = started.get("cwd").and_then(Value::as_str).ok_or_else(|| {
            error(
                "CODEX_PROTOCOL",
                "staged thread did not report its workspace",
            )
        })?;
        if !workspace_matches(cwd, root) {
            return Err(error(
                "WORKSPACE_CONFLICT",
                "staged thread workspace differs from selected context",
            ));
        }
        inject_context(&mut rpc, id, context).await?;
        Ok(id.to_owned())
    }
    .await;
    rpc.finish(result).await
}

/// Import one complete native window, or reopen a receipt-bound destination.
/// Only current runtime options supply capabilities and permissions. No turn
/// is started here. Raw items are serialized directly, without a Value roundtrip.
pub async fn stage_native(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    bundle: &NativeBundle,
    context: &str,
    proxy_url: &str,
    resume_thread: Option<&str>,
) -> Result<StagedThread> {
    stage_native_with_progress(
        executable,
        root,
        options,
        bundle,
        context,
        proxy_url,
        resume_thread,
        |_| Ok(()),
    )
    .await
}

/// Record progress after preflights, immediately before thread/start, and after
/// acknowledged injection but before app-server shutdown. A missing final
/// acknowledgement preserves uncertainty instead of authorizing duplicate work.
#[allow(clippy::too_many_arguments)] // The hook supplements the compatible public staging API.
pub(crate) async fn stage_native_with_progress<F: FnMut(StagingProgress) -> Result<()>>(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    bundle: &NativeBundle,
    context: &str,
    proxy_url: &str,
    resume_thread: Option<&str>,
    mut progress: F,
) -> Result<StagedThread> {
    crate::native_import::validate(bundle)?;
    let compatibility = &bundle.manifest().compatibility;
    verify_native_version(executable, root, &compatibility.runtime_version).await?;
    if resume_thread.is_some_and(|id| !valid_thread_id(id)) {
        return Err(error("CODEX_PROTOCOL", "invalid receipt thread ID"));
    }
    let args = proxy_args(&options, Some(proxy_url))?;
    let mut rpc = Rpc::spawn(executable, root, &args)?;
    // Native payload is at most 8 MiB; the current document prompt is at most
    // 2 MiB before JSON escaping. Responses retain their independent 4 MiB cap.
    rpc.request_limit = 24 * 1024 * 1024;
    let result = async {
        initialize_rpc(&mut rpc).await?;
        if !effective_proxy_matches(&mut rpc, root, proxy_url, &compatibility.provider).await? {
            return Err(error(
                "NATIVE_ROUTE_CONFLICT",
                "effective Codex configuration differs from the owned native proxy route",
            ));
        }
        let started = start_or_resume(
            &mut rpc,
            options.thread_params,
            resume_thread,
            &mut progress,
        )
        .await?;
        let id = thread_id(&started)
            .ok_or_else(|| error("CODEX_PROTOCOL", "invalid staged thread ID"))?;
        if !durable_thread_matches(&started, id, resume_thread) {
            return Err(error(
                "CODEX_PROTOCOL",
                "native thread identity or durable storage differs from requested binding",
            ));
        }
        let cwd = started.get("cwd").and_then(Value::as_str).ok_or_else(|| {
            error(
                "CODEX_PROTOCOL",
                "native thread did not report its workspace",
            )
        })?;
        if !workspace_matches(cwd, root) {
            return Err(error(
                "WORKSPACE_CONFLICT",
                "native thread workspace differs from selected context",
            ));
        }
        let model = started.get("model").and_then(Value::as_str);
        let provider = started.get("modelProvider").and_then(Value::as_str);
        if model != Some(compatibility.model.as_str())
            || provider != Some(compatibility.provider.as_str())
        {
            return Err(error(
                "NATIVE_RUNTIME_MISMATCH",
                "effective model/provider differs from the native bundle",
            ));
        }
        if resume_thread.is_none() {
            #[derive(Serialize)]
            #[serde(rename_all = "camelCase")]
            struct Injection<'a> {
                thread_id: &'a str,
                items: Vec<&'a serde_json::value::RawValue>,
            }
            let current = serde_json::value::to_raw_value(&context_message(context))
                .map_err(|_| error("CODEX_PROTOCOL", "could not encode current project context"))?;
            let mut items: Vec<_> = bundle.raw_items().iter().map(AsRef::as_ref).collect();
            items.push(current.as_ref());
            rpc.request(
                "thread/inject_items",
                Injection {
                    thread_id: id,
                    items,
                },
            )
            .await?;
        }
        let staged = StagedThread {
            thread_id: id.to_owned(),
            model: compatibility.model.clone(),
            model_provider: compatibility.provider.clone(),
        };
        progress(StagingProgress::Injected(staged.clone()))?;
        Ok(staged)
    }
    .await;
    rpc.finish(result).await
}

pub(crate) fn staged_identity(
    value: &Value,
    root: &Path,
    expected_id: Option<&str>,
) -> Result<StagedThread> {
    let id = thread_id(value).ok_or_else(|| error("CODEX_PROTOCOL", "invalid worker thread ID"))?;
    if !durable_thread_matches(value, id, expected_id) {
        return Err(error(
            "CODEX_PROTOCOL",
            "worker identity or durable storage mismatch",
        ));
    }
    let cwd = value
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| error("CODEX_PROTOCOL", "missing worker workspace"))?;
    if !workspace_matches(cwd, root) {
        return Err(error(
            "WORKSPACE_CONFLICT",
            "worker workspace differs from selected worktree",
        ));
    }
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or_else(|| error("CODEX_PROTOCOL", "missing worker model"))?;
    let provider = value
        .get("modelProvider")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= 256)
        .ok_or_else(|| error("CODEX_PROTOCOL", "missing worker provider"))?;
    Ok(StagedThread {
        thread_id: id.into(),
        model: model.into(),
        model_provider: provider.into(),
    })
}

/// Stage a fresh bound worker, or reopen its exact local thread. A context
/// update appends current selected documents, never the original native window.
pub async fn stage_worker(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    proxy: Option<&str>,
    resume: Option<&str>,
    context_update: Option<&str>,
    native_runtime: Option<(&str, &str)>,
) -> Result<StagedThread> {
    stage_worker_with_progress(
        executable,
        root,
        options,
        proxy,
        resume,
        context_update,
        native_runtime,
        |_| Ok(()),
    )
    .await
}

#[allow(clippy::too_many_arguments)] // The hook supplements the compatible public staging API.
pub(crate) async fn stage_worker_with_progress<F: FnMut(StagingProgress) -> Result<()>>(
    executable: &OsStr,
    root: &Path,
    options: StagingOptions,
    proxy: Option<&str>,
    resume: Option<&str>,
    context_update: Option<&str>,
    native_runtime: Option<(&str, &str)>,
    mut progress: F,
) -> Result<StagedThread> {
    verify_native_version(executable, root, "0.154.0").await?;
    if resume.is_some_and(|id| !valid_thread_id(id)) {
        return Err(error("CODEX_PROTOCOL", "invalid bound worker thread ID"));
    }
    if native_runtime.is_some() && proxy.is_none() {
        return Err(error(
            "NATIVE_PROXY_REQUIRED",
            "this worker contains saved native context; pass --proxy",
        ));
    }
    let args = proxy_args(&options, proxy)?;
    let mut rpc = Rpc::spawn(executable, root, &args)?;
    let result = async {
        initialize_rpc(&mut rpc).await?;
        verify_proxy(&mut rpc, root, proxy).await?;
        let started =
            start_or_resume(&mut rpc, options.thread_params, resume, &mut progress).await?;
        let staged = staged_identity(&started, root, resume)?;
        if native_runtime.is_some_and(|(model, provider)| {
            staged.model != model || staged.model_provider != provider
        }) {
            return Err(error(
                "NATIVE_RUNTIME_MISMATCH",
                "effective worker model/provider differs from its native state",
            ));
        }
        if let Some(context) = context_update {
            if resume.is_some() {
                progress(StagingProgress::Injecting)?;
            }
            inject_context(&mut rpc, &staged.thread_id, context).await?;
        }
        progress(StagingProgress::Injected(staged.clone()))?;
        Ok(staged)
    }
    .await;
    rpc.finish(result).await
}
