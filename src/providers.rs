//! Explicit upstream routing; model names never select a credential destination.
use crate::config::Config;
use anyhow::{Context, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Openai,
    Anthropic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    Responses,
    Messages,
    Chat,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    pub name: String,
    pub protocol: Protocol,
    pub upstream: String,
    /// Used only when the caller supplied no credential. Custom routes have no default.
    pub api_key_env: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    providers: Vec<Provider>,
}

pub fn validate_upstream(value: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(value).context("invalid provider upstream URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "provider upstream must be HTTP(S)"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "provider upstream cannot contain credentials, query or fragment"
    );
    Ok(())
}

pub fn load(config: &Config) -> anyhow::Result<Vec<Provider>> {
    let mut providers = vec![
        Provider {
            name: "openai".into(),
            protocol: Protocol::Openai,
            upstream: config.upstream.clone(),
            api_key_env: Some("OPENAI_API_KEY".into()),
        },
        Provider {
            name: "anthropic".into(),
            protocol: Protocol::Anthropic,
            upstream: config.anthropic_upstream.clone(),
            api_key_env: Some("ANTHROPIC_API_KEY".into()),
        },
        Provider {
            name: "codex".into(),
            protocol: Protocol::Openai,
            upstream: "https://chatgpt.com/backend-api/codex".into(),
            api_key_env: None,
        },
    ];
    if let Some(path) = &config.providers {
        ensure!(
            std::fs::metadata(path)?.len() <= 65_536,
            "provider configuration too large"
        );
        let file: File = toml::from_str(&std::fs::read_to_string(path)?)?;
        ensure!(
            file.providers.len() <= 32,
            "at most 32 configured providers"
        );
        let mut seen = std::collections::HashSet::new();
        for provider in file.providers {
            ensure!(
                !provider.name.is_empty()
                    && provider.name.len() <= 64
                    && provider
                        .name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
                "invalid provider name"
            );
            ensure!(
                seen.insert(provider.name.clone()),
                "duplicate configured provider"
            );
            ensure!(
                !matches!(provider.name.as_str(), "openai" | "codex")
                    || provider.protocol == Protocol::Openai,
                "openai/codex routes require the openai protocol"
            );
            ensure!(
                provider.name != "anthropic" || provider.protocol == Protocol::Anthropic,
                "anthropic route requires the anthropic protocol"
            );
            validate_upstream(&provider.upstream)?;
            if let Some(env) = &provider.api_key_env {
                ensure!(
                    !env.is_empty()
                        && env.len() <= 128
                        && env.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
                    "invalid credential environment variable name"
                );
            }
            if let Some(old) = providers.iter_mut().find(|p| p.name == provider.name) {
                *old = provider;
            } else {
                providers.push(provider);
            }
        }
    }
    Ok(providers)
}
