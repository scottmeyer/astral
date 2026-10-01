//! Retrieval client and small stdio MCP adapter. No execution or provider credentials.
use anyhow::{Context, ensure};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

pub async fn fetch(
    proxy: &str,
    handle: &str,
    offset: usize,
    limit: usize,
    query: Option<&str>,
) -> anyhow::Result<Value> {
    ensure!(
        crate::archive::valid_handle(handle),
        "invalid artifact handle"
    );
    ensure!(
        limit > 0 && limit <= crate::archive::PAGE_LIMIT,
        "limit must be 1..16384 bytes"
    );
    if let Some(query) = query {
        ensure!(
            !query.is_empty() && query.len() <= 1024,
            "query must be 1..1024 bytes"
        );
    }
    crate::providers::validate_upstream(proxy)?;
    let mut url = reqwest::Url::parse(proxy)?;
    ensure!(
        url.path() == "/" || url.path().is_empty(),
        "proxy URL must name the listener root, without a provider path"
    );
    url.set_path(&format!("/_astral/artifacts/{handle}"));
    url.query_pairs_mut()
        .append_pair("offset", &offset.to_string())
        .append_pair("limit", &limit.to_string());
    if let Some(query) = query {
        url.query_pairs_mut().append_pair("query", query);
    }
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .get(url)
        .send()
        .await
        .map_err(|e| e.without_url())?;
    ensure!(
        response.status().is_success(),
        "artifact retrieval failed (HTTP {})",
        response.status().as_u16()
    );
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.without_url())?;
        ensure!(
            bytes.len().saturating_add(chunk.len()) <= 131_072,
            "retrieval response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).context("invalid retrieval response")
}

pub(crate) fn definitions() -> Value {
    json!({"tools":[
        {"name":"astral_recall","description":"Retrieve exact original tool output archived by the Astral proxy. Use a handle from an archived-output marker. Returns a byte page; continue with next_offset. Does not rerun any tool.","inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":16384}},"required":["handle"],"additionalProperties":false}},
        {"name":"astral_search","description":"Find literal text in an archived tool output. Returns byte offsets for astral_recall. Use offset to paginate results. Does not rerun tools.","inputSchema":{"type":"object","properties":{"handle":{"type":"string"},"query":{"type":"string","minLength":1,"maxLength":1024},"offset":{"type":"integer","minimum":0}},"required":["handle","query"],"additionalProperties":false}}
    ]})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    handle: String,
    #[serde(default)]
    offset: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    query: Option<String>,
}
fn default_limit() -> usize {
    4096
}

pub(crate) async fn dispatch(proxy: &str, request: Value, initialized: &mut bool) -> Option<Value> {
    let id = request.get("id")?.clone();
    let error =
        |code, message| json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}});
    if request["jsonrpc"] != "2.0" || !(id.is_string() || id.is_number()) {
        return Some(error(-32600, "Invalid Request"));
    }
    let result = match request["method"].as_str().unwrap_or("") {
        "initialize" => {
            *initialized = true;
            json!({"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":false}},"serverInfo":{"name":"astral-recall","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        _ if !*initialized => return Some(error(-32002, "Initialize first")),
        "tools/list" => definitions(),
        "tools/call" => {
            let name = request["params"]["name"].as_str().unwrap_or("");
            if !matches!(name, "astral_recall" | "astral_search") {
                return Some(error(-32602, "Unknown tool"));
            }
            let args: Arguments =
                match serde_json::from_value(request["params"]["arguments"].clone()) {
                    Ok(args) => args,
                    Err(_) => return Some(error(-32602, "Invalid tool arguments")),
                };
            if (name == "astral_search") != args.query.is_some() {
                return Some(error(-32602, "query is required only for astral_search"));
            }
            match fetch(
                proxy,
                &args.handle,
                args.offset,
                args.limit,
                args.query.as_deref(),
            )
            .await
            {
                Ok(value) => {
                    json!({"content":[{"type":"text","text":value.to_string()}],"isError":false})
                }
                Err(e) => json!({"content":[{"type":"text","text":e.to_string()}],"isError":true}),
            }
        }
        _ => return Some(error(-32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

pub async fn serve(proxy: &str) -> anyhow::Result<()> {
    crate::providers::validate_upstream(proxy)?;
    let mut input = tokio::io::BufReader::new(tokio::io::stdin());
    let mut output = tokio::io::stdout();
    let mut initialized = false;
    loop {
        let mut line = Vec::new();
        let count = (&mut input)
            .take(65_537)
            .read_until(b'\n', &mut line)
            .await?;
        if count == 0 {
            return Ok(());
        }
        ensure!(line.len() <= 65_536, "MCP request exceeds 64 KiB");
        let response = match serde_json::from_slice::<Value>(&line) {
            Ok(value) => dispatch(proxy, value, &mut initialized).await,
            Err(_) => Some(
                json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}),
            ),
        };
        if let Some(value) = response {
            output.write_all(value.to_string().as_bytes()).await?;
            output.write_all(b"\n").await?;
            output.flush().await?;
        }
    }
}
