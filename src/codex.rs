//! Owned, bounded Codex app-server adapter for staging project context.
//!
//! The interactive Codex process owns execution, authentication, approvals and
//! sandbox enforcement. Staging does not execute tools or start inference turns.

mod options;
mod rpc;
mod staging;

pub use options::StagingOptions;
pub use rpc::wait_interactive;
pub use staging::{StagedThread, stage_fresh, stage_native, stage_worker};

pub(crate) use rpc::{Rpc, initialize_rpc, verify_native_version};
pub(crate) use staging::{
    StagingProgress, proxy_args, stage_native_with_progress, stage_worker_with_progress,
    staged_identity, verify_proxy,
};

pub(crate) fn error(code: &'static str, message: impl Into<String>) -> crate::project::Error {
    crate::project::Error {
        code,
        message: message.into(),
    }
}
