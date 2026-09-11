//! API Platform policy as documented 2026-09-10. Unknown models/wires fail neutral.
use crate::config::Retention;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Generation {
    Modern,
    Legacy,
    Legacy24Only,
    Unknown,
}

fn family(model: &str, name: &str) -> bool {
    model == name || model.strip_prefix(name).is_some_and(|s| s.starts_with('-'))
}

pub fn generation(model: &str) -> Generation {
    if family(model, "gpt-5.6") || family(model, "gpt-6-astra") {
        Generation::Modern
    } else if family(model, "gpt-5.5") {
        Generation::Legacy24Only
    } else if ["gpt-5.4", "gpt-5.2", "gpt-5.1", "gpt-4.1"]
        .iter()
        .any(|f| family(model, f))
        || matches!(model, "gpt-5" | "gpt-5-codex")
    {
        Generation::Legacy
    } else {
        Generation::Unknown
    }
}

pub fn apply(
    request: &mut Value,
    platform: bool,
    retention: Retention,
    lane: Option<&str>,
) -> anyhow::Result<()> {
    let obj = request
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("request must be a JSON object"))?;
    if let Some(lane) = lane {
        if !obj.contains_key("prompt_cache_key") {
            obj.insert(
                "prompt_cache_key".into(),
                json!(format!("astral:{}", &lane[..32])),
            );
        }
    }
    // Compatibility backends do not inherit Platform's accepted parameter set.
    if !platform {
        anyhow::ensure!(
            retention == Retention::ProviderDefault,
            "retention overrides require the official API Platform upstream"
        );
        return Ok(());
    }
    // Caller controls win verbatim, even if the configured default differs.
    if obj.contains_key("prompt_cache_retention") || obj.contains_key("prompt_cache_options") {
        return Ok(());
    }
    let model = obj.get("model").and_then(Value::as_str).unwrap_or("");
    match (generation(model), retention) {
        (Generation::Modern, Retention::ProviderDefault) => {
            obj.insert(
                "prompt_cache_options".into(),
                json!({"mode":"implicit", "ttl":"30m"}),
            );
        }
        (Generation::Modern, _) => anyhow::bail!(
            "this model uses prompt_cache_options.ttl=30m, not legacy retention; use provider-default"
        ),
        (Generation::Legacy | Generation::Legacy24Only, Retention::Extended) => {
            obj.insert("prompt_cache_retention".into(), json!("24h"));
        }
        (Generation::Legacy, Retention::InMemory) => {
            obj.insert("prompt_cache_retention".into(), json!("in_memory"));
        }
        (Generation::Legacy24Only, Retention::InMemory) => {
            anyhow::bail!("GPT-5.5 supports 24h retention only")
        }
        (Generation::Unknown, Retention::Extended | Retention::InMemory) => {
            anyhow::bail!("unknown model retention capability; set caller parameters explicitly")
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_cache_keys_use_the_astral_namespace() {
        let mut request = json!({"model":"gpt-6-astra"});
        let lane = "a".repeat(64);
        apply(&mut request, false, Retention::ProviderDefault, Some(&lane)).unwrap();
        assert_eq!(
            request["prompt_cache_key"],
            format!("astral:{}", "a".repeat(32))
        );
    }

    #[test]
    fn legacy_modern_and_unknown_have_distinct_controls() {
        for (model, expected) in [
            ("gpt-5.5", "prompt_cache_retention"),
            ("gpt-5.6", "prompt_cache_options"),
            ("gpt-6-astra", "prompt_cache_options"),
        ] {
            let mut r = json!({"model":model});
            let retention = if model == "gpt-5.5" {
                Retention::Extended
            } else {
                Retention::ProviderDefault
            };
            apply(&mut r, true, retention, None).unwrap();
            assert!(r.get(expected).is_some());
        }
        let mut r = json!({"model":"gpt-future"});
        apply(&mut r, true, Retention::ProviderDefault, None).unwrap();
        assert_eq!(r, json!({"model":"gpt-future"}));
        assert!(
            apply(
                &mut json!({"model":"gpt-5.5"}),
                true,
                Retention::InMemory,
                None
            )
            .is_err()
        );
    }
    #[test]
    fn caller_and_compatible_wire_are_preserved() {
        let mut r = json!({"model":"gpt-5.6", "prompt_cache_options":{"mode":"explicit"}, "prompt_cache_key":"caller"});
        let original = r.clone();
        apply(&mut r, true, Retention::Extended, Some(&"a".repeat(64))).unwrap();
        assert_eq!(r, original);
        let mut r = json!({"model":"gpt-5.5"});
        apply(&mut r, false, Retention::ProviderDefault, None).unwrap();
        assert_eq!(r, json!({"model":"gpt-5.5"}));
    }
}
