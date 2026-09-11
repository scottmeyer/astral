use astral::usage::Usage;
use clap::Parser;
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::PathBuf,
};

#[derive(Parser)]
#[command(
    about = "Aggregate observed response and compaction usage separately; no inferred dollar savings"
)]
struct Args {
    #[arg(long, default_value = ".astral-runtime/ledger.jsonl")]
    ledger: PathBuf,
}

#[derive(Default, Serialize)]
struct Totals {
    calls: u64,
    usage_calls: u64,
    input_tokens_inclusive: u64,
    cached_tokens: u64,
    reported_cache_write_tokens: u64,
    calls_without_cache_write_reporting: u64,
    output_tokens: u64,
    input_bytes: u64,
    output_bytes: u64,
    cache_hit_rate: Option<f64>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let reader = BufReader::new(std::fs::File::open(args.ledger)?);
    let mut totals: BTreeMap<String, Totals> = BTreeMap::new();
    let mut malformed_lines = 0u64;
    for line in reader.lines() {
        let line = line?;
        let Ok(row) = serde_json::from_str::<Value>(&line) else {
            malformed_lines += 1;
            continue;
        };
        let kind = row["kind"].as_str().unwrap_or("unknown").to_owned();
        let group = totals.entry(kind).or_default();
        group.calls += 1;
        group.input_bytes += row
            .get("bytes_in")
            .or_else(|| row.get("input_bytes"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        group.output_bytes += row
            .get("bytes_out")
            .or_else(|| row.get("output_bytes"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if let Ok(usage) = serde_json::from_value::<Usage>(row["usage"].clone()) {
            group.usage_calls += 1;
            group.input_tokens_inclusive += usage.input_tokens;
            group.cached_tokens += usage.cached_tokens;
            group.output_tokens += usage.output_tokens;
            match usage.cache_write_tokens {
                Some(w) => group.reported_cache_write_tokens += w,
                None => group.calls_without_cache_write_reporting += 1,
            }
        }
    }
    for total in totals.values_mut() {
        total.cache_hit_rate = (total.input_tokens_inclusive > 0)
            .then(|| total.cached_tokens as f64 / total.input_tokens_inclusive as f64);
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"groups":totals,"malformed_lines":malformed_lines})
        )?
    );
    Ok(())
}
