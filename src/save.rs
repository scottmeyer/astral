//! Explicit native save of an exclusively bound, stopped Astral worker.
use crate::codex::{
    Rpc, StagingOptions, error, initialize_rpc, proxy_args, staged_identity, verify_native_version,
    verify_proxy,
};
use crate::managed_proxy::ManagedProxy;
use crate::native_bundle::{
    BundleManifest, Capture, Compatibility, NativeBundle, PayloadManifest, Source,
};
use crate::project::{Project, Result};
use crate::workspace::WorktreeBinding;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub struct SaveArguments {
    pub root: PathBuf,
    pub name: String,
    pub context: String,
    pub work: String,
    pub proxy: bool,
    pub codex_args: Vec<OsString>,
}

pub async fn save(request: SaveArguments) -> Result<Value> {
    if !request.proxy {
        return Err(error(
            "NATIVE_PROXY_REQUIRED",
            "native save requires explicit --proxy",
        ));
    }
    let source = request
        .root
        .canonicalize()
        .map_err(|_| error("INVALID_ROOT", "repository root unavailable"))?;
    let project_id = Project::load(&source)?.project_id().to_owned();
    let mut binding =
        WorktreeBinding::open_existing(&source, &project_id, &request.context, &request.work)?;
    let root = binding.root().to_owned();
    let mut metadata = binding.worker_metadata().clone();
    let thread = metadata
        .thread_id
        .clone()
        .ok_or_else(|| error("WORKSPACE_NO_WORKER", "this worktree has no staged worker"))?;
    if metadata.model.as_deref() != Some("gpt-6-astra")
        || metadata.provider.as_deref() != Some("openai")
    {
        return Err(error(
            "NATIVE_RUNTIME_MISMATCH",
            "native save currently requires gpt-6-astra on the OpenAI Responses Lite route",
        ));
    }
    let selected = Project::load(&root)?.launch_context(&request.context, Some(&request.work))?;
    let selected_hash = selected
        .native
        .as_ref()
        .map(|b| b.summary().manifest_sha256);
    if !metadata.accepts_selected_bundle(selected_hash.as_deref()) {
        return Err(error(
            "WORKSPACE_SEED_CHANGED",
            "selected native history differs from this worker's recorded selection; use a new work ID",
        ));
    }
    let context = selected.current_context;
    crate::projection_save::validate_target(&root, &request.name, &context.selection.subsystems)?;
    let text = crate::worker_launch::context_text(&root, &context)?;
    StagingOptions::validate_initialization(&root, &request.codex_args)?;
    let options = StagingOptions::from_args(&root, &request.codex_args)?;
    let program = crate::launch::executable();
    verify_native_version(&program, &root, "0.154.0").await?;
    let proxy = ManagedProxy::start(binding.proxy_state_dir()).await?;
    let result = async {
        let args = proxy_args(&options, Some(proxy.base_url()))?;
        let mut rpc = Rpc::spawn(&program, &root, &args)?;
        let captured = async {
            initialize_rpc(&mut rpc).await?;
            verify_proxy(&mut rpc, &root, Some(proxy.base_url())).await?;
            let mut params = options.thread_params;
            params["threadId"] = json!(thread);
            params["excludeTurns"] = json!(true);
            let response = rpc.request("thread/resume", params).await?;
            let identity = staged_identity(&response, &root, Some(&thread))?;
            if Some(identity.model.as_str()) != metadata.model.as_deref() || Some(identity.model_provider.as_str()) != metadata.provider.as_deref() {
                return Err(error("NATIVE_RUNTIME_MISMATCH", "save runtime differs from the bound worker"));
            }
            if metadata.selection_digest.as_deref() != Some(context.selection_digest.as_str()) {
                rpc.request("thread/inject_items", json!({"threadId":thread,"items":[{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}]})).await?;
            }
            let read = rpc.request("thread/read", json!({"threadId":thread,"includeTurns":false})).await?;
            let path = rollout_path(&read, &thread)?;
            let before = crate::native_capture::read_snapshot(&path, &thread)?;
            // A failed or cancelled save may still have installed a checkpoint.
            // Persist this conservative requirement before triggering compaction.
            metadata.requires_tool_rebinding = true;
            metadata.selection_digest = Some(context.selection_digest.clone());
            binding.update_worker_metadata(metadata.clone())?;
            eprintln!("Saving Astral work {} from {}; waiting for native compaction.", request.work, root.display());
            let (turn, item) = rpc.compact(&thread).await?;
            // Recheck the runtime's source association before closing it.
            let read = rpc.request("thread/read", json!({"threadId":thread,"includeTurns":false})).await?;
            if rollout_path(&read, &thread)? != path { return Err(error("CAPTURE_SOURCE_CHANGED", "worker rollout path changed during capture")); }
            Ok((before, path, turn, item))
        }.await;
        let closed = rpc.close().await;
        let (before, path, turn, item) = captured?;
        closed?;
        let after = crate::native_capture::read_snapshot(&path, &thread)?;
        let capture = crate::native_capture::capture(&before, &after, &thread, &turn, &item)?;
        if Project::load(&root)?.launch_context(&request.context, Some(&request.work))?.current_context.selection_digest != context.selection_digest {
            return Err(error("CAPTURE_CONTEXT_CHANGED", "selected repository context changed during native capture; retry save after reviewing the changes"));
        }
        let (revision, dirty) = crate::git_context::revision(&root).await?;
        let mut parents: Vec<_> = [&metadata.seed_bundle_sha256, &metadata.saved_bundle_sha256].into_iter().flatten().cloned().collect();
        parents.sort(); parents.dedup();
        let manifest = BundleManifest {
            schema_version:1, format:"astral-codex-native".into(),
            payload:PayloadManifest { file:"window.json".into(), sha256:crate::hash(&capture.payload_bytes), bytes:capture.payload_bytes.len(), item_count:capture.item_count },
            compatibility:Compatibility {runtime:"codex".into(), runtime_version:"0.154.0".into(), protocol:"openai-responses-lite".into(), provider:"openai".into(), model:"gpt-6-astra".into(), requires_tool_rebinding:true, identity_scope:"same-account".into()},
            source:Source { project_id:project_id.clone(), revision:Some(revision), dirty, selection_sha256:context.selection_digest.clone(), history_sha256:capture.source_history_sha256 },
            capture:Capture { boundary:"completed-compaction".into(), history_complete:true, last_checkpoint_index:capture.last_checkpoint_index }, parents,
        };
        let bytes = serde_json::to_vec_pretty(&manifest).map_err(|_| error("CAPTURE_ENCODING", "could not encode bundle manifest"))?;
        let bundle = NativeBundle::validate(&bytes, &capture.payload_bytes)?;
        crate::native_import::validate(&bundle)?;
        let publication = crate::projection_save::publish(&root, &request.name, &context.selection.subsystems, &bundle, None)?;
        let saved_hash = bundle.summary().manifest_sha256;
        metadata.saved_bundle_sha256 = Some(saved_hash.clone());
        metadata.selection_digest = Some(context.selection_digest);
        // Saving under a different name must not replace the selected projection's
        // anchor. Resolve again because this publication may have advanced it.
        let recorded = (|| {
            let after = Project::load(&root)?.launch_context(&request.context, Some(&request.work))?;
            let after_hash = after.native.as_ref().map(|b| b.summary().manifest_sha256);
            if after_hash != selected_hash && after_hash.as_deref() != Some(saved_hash.as_str()) {
                return Err(error("WORKSPACE_SEED_CHANGED", "selected native history changed during publication"));
            }
            metadata.selected_bundle_sha256 = after_hash;
            metadata.selected_bundle_recorded = true;
            binding.update_worker_metadata(metadata)
        })();
        if recorded.is_err() {
            eprintln!("{}", json!({"operation":"save", "publication":publication,"worktree":root,"worker_metadata_updated":false}));
            return Err(error("SAVE_PUBLISHED_METADATA_FAILED", "the native projection was published, but the private worker receipt could not be updated; retain the reported artifacts and inspect the binding before retrying; a new work ID can import the published projection"));
        }
        Ok(json!({"operation":"save", "work":request.work,"worktree":root,"branch":binding.branch(),"publication":publication,"commit_required":true}))
    }.await;
    let stopped = proxy.stop().await;
    let receipt = result?;
    stopped?;
    Ok(receipt)
}

fn rollout_path(response: &Value, thread: &str) -> Result<PathBuf> {
    if response.pointer("/thread/id").and_then(Value::as_str) != Some(thread) {
        return Err(error(
            "CAPTURE_SOURCE_CHANGED",
            "runtime returned another worker",
        ));
    }
    let value = response
        .pointer("/thread/path")
        .and_then(Value::as_str)
        .filter(|v| v.len() <= 4096)
        .ok_or_else(|| {
            error(
                "CAPTURE_SOURCE_UNAVAILABLE",
                "runtime did not expose a local rollout path",
            )
        })?;
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(error(
            "CAPTURE_SOURCE_UNAVAILABLE",
            "runtime rollout path must be absolute",
        ));
    }
    Ok(path.to_owned())
}
