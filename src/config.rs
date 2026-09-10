use clap::{Parser, ValueEnum};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Mode {
    Rolling,
    Passthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Retention {
    ProviderDefault,
    InMemory,
    Extended,
}

#[derive(Debug, Clone, Parser)]
#[command(version, about)]
pub struct Config {
    #[arg(long, default_value = "127.0.0.1:8088")]
    pub listen: SocketAddr,
    /// API base including its version/path, without the /responses suffix.
    #[arg(
        long,
        env = "OSTK_GPT_UPSTREAM",
        default_value = "https://api.openai.com/v1"
    )]
    pub upstream: String,
    #[arg(long, value_enum, default_value = "rolling")]
    pub mode: Mode,
    #[arg(long, default_value = ".ostk-gpt")]
    pub state_dir: PathBuf,
    /// Explicit opt-in to POST /responses/compact on a compatible upstream.
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
        let url = reqwest::Url::parse(&self.upstream)?;
        anyhow::ensure!(
            matches!(url.scheme(), "http" | "https"),
            "upstream must be HTTP(S)"
        );
        anyhow::ensure!(url.host_str().is_some(), "upstream must have a host");
        anyhow::ensure!(
            url.username().is_empty() && url.password().is_none(),
            "use Authorization headers, not URL credentials"
        );
        anyhow::ensure!(
            url.query().is_none() && url.fragment().is_none(),
            "upstream cannot contain a query or fragment"
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
