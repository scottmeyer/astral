use clap::{Parser, ValueEnum};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    /// Archive older tool output without requiring provider-native compaction.
    Tools,
    Rolling,
    Passthrough,
}

/// Experimental, trusted-local-Codex-only compatibility lane; never rolls history.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NativeToolBindingMode {
    #[default]
    Disabled,
    Observe,
    RepeatBefore,
    Rebind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionBackend {
    Standalone,
    Inline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Retention {
    ProviderDefault,
    InMemory,
    Extended,
}

#[derive(Debug, Clone, Parser)]
#[command(name = "astral", version, about)]
pub struct Config {
    #[arg(long, default_value = "127.0.0.1:8088")]
    pub listen: SocketAddr,
    /// API base including its version/path, without the /responses suffix.
    #[arg(
        long,
        env = "ASTRAL_UPSTREAM",
        default_value = "https://api.openai.com/v1"
    )]
    pub upstream: String,
    /// Anthropic API base, including /v1. Served alongside the OpenAI routes.
    #[arg(
        long,
        env = "ASTRAL_ANTHROPIC_UPSTREAM",
        default_value = "https://api.anthropic.com/v1"
    )]
    pub anthropic_upstream: String,
    /// Optional TOML file containing named [[providers]] routes.
    #[arg(long, env = "ASTRAL_PROVIDERS")]
    pub providers: Option<PathBuf>,
    /// Minimum text tool-result size eligible for archival.
    #[arg(long, default_value_t = 16_384)]
    pub tool_result_bytes: usize,
    /// Total head/tail preview bytes retained for an archived result.
    #[arg(long, default_value_t = 2048)]
    pub tool_preview_bytes: usize,
    /// Keep this many recent tool results inside a long turn; zero disables intra-turn archival.
    #[arg(long, default_value_t = 4)]
    pub keep_recent_tool_results: usize,
    /// Archive capacity. Full archives preserve original input instead of evicting artifacts.
    #[arg(long, default_value_t = 536_870_912)]
    pub archive_max_bytes: u64,
    /// Additional trusted PEM certificates, including managed local proxy CAs.
    #[arg(long, env = "SSL_CERT_FILE")]
    pub upstream_ca_bundle: Option<PathBuf>,
    /// Native compact route relative to the upstream base.
    #[arg(long, default_value = "/responses/compact")]
    pub compact_path: String,
    /// Inline lets Astral schedule native checkpoints within generation responses.
    #[arg(long, value_enum, default_value = "standalone")]
    pub compaction_backend: CompactionBackend,
    /// Provider token threshold when an inline rollover is scheduled.
    #[arg(long, default_value_t = 8192)]
    pub inline_threshold_tokens: usize,
    /// Allow native inline checkpoints after complete external tool results.
    #[arg(long)]
    pub inline_tool_boundaries: bool,
    /// Honor x-astral-roll-estimate when supplied; otherwise retain byte scheduling.
    #[arg(long)]
    pub economic_roll_policy: bool,
    #[arg(long, value_enum, default_value = "tools")]
    pub mode: Mode,
    /// Native checkpoint tool binding. Observe is an unmodified transport control.
    #[arg(long, alias = "ast000-compat", value_enum, default_value = "disabled")]
    pub native_tool_binding: NativeToolBindingMode,
    #[arg(
        long,
        env = "ASTRAL_PROXY_STATE_DIR",
        default_value = ".astral-runtime"
    )]
    pub state_dir: PathBuf,
    /// Explicit opt-in to native compaction on a compatible upstream.
    #[arg(long)]
    pub allow_compatible_compaction: bool,
    /// Preserve the provider/org default unless explicitly requested.
    #[arg(long, value_enum, default_value = "provider-default")]
    pub retention: Retention,
    /// Conservative byte-based trigger, not a model context-window limit.
    #[arg(long, default_value_t = 160_000)]
    pub roll_bytes: usize,
    #[arg(long, default_value_t = 2)]
    pub keep_recent_turns: usize,
    #[arg(long, default_value_t = 32_000)]
    pub min_compact_bytes: usize,
    /// Optional inactivity trigger. Zero disables; never treated as a cache expiry guarantee.
    #[arg(long, default_value_t = 0)]
    pub idle_roll_seconds: u64,
    #[arg(long, default_value_t = 60)]
    pub min_roll_seconds: u64,
    /// Minimum byte reduction to accept a compacted window (0..1).
    #[arg(long, default_value_t = 0.15)]
    pub min_savings: f64,
    #[arg(long, default_value_t = 32 * 1024 * 1024)]
    pub max_body_bytes: usize,
    #[arg(long, default_value_t = 16 * 1024 * 1024)]
    pub max_observation_bytes: usize,
    #[arg(long, default_value_t = 1024)]
    pub max_sessions: usize,
    #[arg(long, default_value_t = 180)]
    pub compact_timeout_seconds: u64,
    #[arg(long, default_value_t = 900)]
    pub request_timeout_seconds: u64,
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        crate::providers::validate_upstream(&self.anthropic_upstream)?;
        anyhow::ensure!(
            self.tool_preview_bytes >= 128
                && self.tool_preview_bytes.saturating_add(1024) < self.tool_result_bytes,
            "tool-result-bytes must exceed tool-preview-bytes by more than 1024; preview must be at least 128"
        );
        anyhow::ensure!(
            self.archive_max_bytes > 0,
            "archive-max-bytes must be positive"
        );
        let url = reqwest::Url::parse(&self.upstream)?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https"),
            "upstream must be HTTP(S)"
        );
        anyhow::ensure!(url.host_str().is_some(), "upstream must have a host");
        if self.native_tool_binding != NativeToolBindingMode::Disabled {
            anyhow::ensure!(
                self.mode == Mode::Passthrough,
                "native tool binding requires --mode passthrough"
            );
            anyhow::ensure!(
                self.listen.ip().is_loopback(),
                "native tool binding accepts trusted loopback clients only"
            );
            anyhow::ensure!(
                url.scheme() == "https"
                    || matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
                "native tool binding upstream requires TLS or a loopback test server"
            );
        }
        anyhow::ensure!(
            url.username().is_empty() && url.password().is_none(),
            "use Authorization headers, not URL credentials"
        );
        anyhow::ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "upstream cannot contain a query or fragment"
        );
        anyhow::ensure!(
            self.compact_path.starts_with('/')
                && !self.compact_path.starts_with("//")
                && !self.compact_path.contains(['?', '#', '\\']),
            "compact-path must be a path starting with one slash, without query or fragment"
        );
        anyhow::ensure!(
            self.keep_recent_turns > 0,
            "keep-recent-turns must be positive"
        );
        anyhow::ensure!(
            self.roll_bytes > 0 && self.max_body_bytes > self.roll_bytes,
            "roll-bytes must be positive and below max-body-bytes"
        );
        anyhow::ensure!(
            self.min_compact_bytes > 0 && self.max_sessions > 0 && self.max_observation_bytes > 0,
            "limits must be positive"
        );
        anyhow::ensure!(
            (0.0..1.0).contains(&self.min_savings),
            "min-savings must be in [0, 1)"
        );
        anyhow::ensure!(
            self.request_timeout_seconds > 0 && self.compact_timeout_seconds > 0,
            "timeouts must be positive"
        );
        anyhow::ensure!(
            self.inline_threshold_tokens > 0,
            "inline threshold must be positive"
        );
        Ok(())
    }
    pub fn platform(&self) -> bool {
        reqwest::Url::parse(&self.upstream).is_ok_and(|u| {
            u.scheme() == "https"
                && u.host_str() == Some("api.openai.com")
                && u.port_or_known_default() == Some(443)
        })
    }
}
