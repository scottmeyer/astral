//! Preserve unknown JSON values as raw lexemes while changing exact owned groups.
use super::*;
use serde::de::{MapAccess, Visitor};
use serde_json::value::RawValue;
use std::fmt;

#[derive(Default, Serialize)]
struct Object(BTreeMap<String, Box<RawValue>>);
impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Objects;
        impl<'de> Visitor<'de> for Objects {
            type Value = Object;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a bounded object without duplicate keys")
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut result = BTreeMap::new();
                while let Some((key, value)) = map.next_entry::<String, Box<RawValue>>()? {
                    if result.len() >= 256
                        || key.len() > 1024
                        || result.insert(key, value).is_some()
                    {
                        return Err(serde::de::Error::custom(
                            "object exceeds limits or duplicates a key",
                        ));
                    }
                }
                Ok(Object(result))
            }
        }
        d.deserialize_map(Objects)
    }
}
fn parse<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    serde_json::from_slice(bytes).map_err(|_| {
        error(
            "HOOK_CODEX_JSON",
            "Codex hooks require bounded JSON objects and event arrays without duplicate keys",
        )
    })
}
fn raw<T: Serialize>(v: &T) -> Result<Box<RawValue>> {
    serde_json::value::to_raw_value(v)
        .map_err(|_| error("HOOK_ENCODING", "cannot encode Codex hook groups"))
}
pub(super) fn groups() -> BTreeMap<String, String> {
    ["SessionStart","UserPromptSubmit","Stop","PostCompact"].into_iter().map(|event|{
        let handler=json!({"type":"command","command":"astral hook codex","timeout":5,"statusMessage":"Astral context advisory"});
        let group=if event=="SessionStart"{json!({"matcher":"^(startup|resume|compact)$","hooks":[handler]})}else{json!({"hooks":[handler]})};
        (event.into(),serde_json::to_string(&group).expect("fixed group serializes"))
    }).collect()
}
pub(super) struct Transform {
    pub bytes: Option<Vec<u8>>,
    pub blockers: Vec<String>,
}
pub(super) fn transform(
    before: Option<&[u8]>,
    owned: Option<&BTreeMap<String, String>>,
    desired: &BTreeMap<String, String>,
    action: Action,
) -> Result<Transform> {
    let Some(before) = before else {
        if action == Action::Uninstall {
            return Ok(Transform {
                bytes: None,
                blockers: Vec::new(),
            });
        }
        let events: Object = Object(
            desired
                .iter()
                .map(|(event, group)| {
                    Ok((
                        event.clone(),
                        raw(&vec![RawValue::from_string(group.clone()).map_err(
                            |_| error("HOOK_REGISTRATION_INVALID", "invalid owned group"),
                        )?])?,
                    ))
                })
                .collect::<Result<_>>()?,
        );
        let top = Object(BTreeMap::from([("hooks".into(), raw(&events)?)]));
        return Ok(Transform {
            bytes: Some(encode(&top)?),
            blockers: Vec::new(),
        });
    };
    if before.len() > MAX_BYTES {
        return Err(error(
            "HOOK_LIMIT",
            "Codex hook configuration exceeds byte limit",
        ));
    }
    let mut top: Object = parse(before)?;
    let mut events: Object = match top.0.get("hooks") {
        Some(v) => parse(v.get().as_bytes())?,
        None => Object::default(),
    };
    let mut blockers = Vec::new();
    let mut changed = false;
    for (event, group) in desired {
        let mut entries: Vec<Box<RawValue>> = match events.0.get(event) {
            Some(v) => parse(v.get().as_bytes())?,
            None => Vec::new(),
        };
        if entries.len() > 256 {
            return Err(error(
                "HOOK_LIMIT",
                "Codex event exceeds matcher group limit",
            ));
        }
        let previous = owned.and_then(|o| o.get(event));
        let exact: Vec<_> = entries
            .iter()
            .enumerate()
            .filter(|(_, v)| Some(v.get()) == previous.map(String::as_str))
            .map(|(i, _)| i)
            .collect();
        if exact.len() > 1 {
            blockers.push(format!("AMBIGUOUS_OWNED_CODEX_GROUP:{event}"));
            continue;
        }
        match action {
            Action::Install => {
                if let Some(previous) = previous {
                    if exact.len() != 1 || previous != group {
                        blockers.push(format!("CHANGED_OWNED_CODEX_GROUP:{event}"));
                        continue;
                    }
                } else if entries.iter().any(|v| v.get() == group) {
                    blockers.push(format!("UNOWNED_CODEX_GROUP:{event}"));
                    continue;
                } else {
                    entries.push(
                        RawValue::from_string(group.clone())
                            .map_err(|_| error("HOOK_ENCODING", "invalid generated group"))?,
                    );
                    changed = true;
                }
            }
            Action::Uninstall => {
                if let Some(index) = exact.first() {
                    entries.remove(*index);
                    changed = true;
                }
            }
        }
        if changed || events.0.contains_key(event) {
            events.0.insert(event.clone(), raw(&entries)?);
        }
    }
    if !changed {
        return Ok(Transform {
            bytes: Some(before.to_vec()),
            blockers,
        });
    }
    top.0.insert("hooks".into(), raw(&events)?);
    let bytes = encode(&top)?;
    if bytes.len() > MAX_BYTES {
        return Err(error(
            "HOOK_LIMIT",
            "updated Codex hook configuration exceeds byte limit",
        ));
    }
    Ok(Transform {
        bytes: Some(bytes),
        blockers,
    })
}

/// Exact generated groups may be committed and present without local ownership.
/// Configuration observation does not claim that Codex trusts or executes it.
pub(super) fn observe(before: Option<&[u8]>) -> Result<BTreeMap<String, usize>> {
    let desired = groups();
    let Some(before) = before else {
        return Ok(desired.into_keys().map(|key| (key, 0)).collect());
    };
    let top: Object = parse(before)?;
    let events: Object = match top.0.get("hooks") {
        Some(raw) => parse(raw.get().as_bytes())?,
        None => Object::default(),
    };
    desired
        .into_iter()
        .map(|(event, group)| {
            let entries: Vec<Box<RawValue>> = match events.0.get(&event) {
                Some(raw) => parse(raw.get().as_bytes())?,
                None => Vec::new(),
            };
            if entries.len() > 256 {
                return Err(error(
                    "HOOK_LIMIT",
                    "Codex event exceeds matcher group limit",
                ));
            }
            let count = entries.iter().filter(|entry| entry.get() == group).count();
            Ok((event, count))
        })
        .collect()
}
