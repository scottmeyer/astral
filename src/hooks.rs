//! Optional adapters around the shared read-only lifecycle checks.
pub mod codex;
mod dispatch;
pub mod install;
pub(crate) mod storage;

pub use dispatch::{check_callback, codex_callback, git_callback};

pub const GIT_EVENTS: [&str; 6] = [
    "pre-commit",
    "pre-merge-commit",
    "post-commit",
    "post-checkout",
    "post-merge",
    "post-rewrite",
];
