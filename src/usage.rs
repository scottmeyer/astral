use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u64,
    pub cached_tokens: u64,
    // None means not reported, not an assumed zero-priced write.
    pub cache_write_tokens: Option<u64>,
    pub output_tokens: u64,
}
impl Usage {
    pub fn parse(v: &Value) -> Option<Self> {
        let u = Self {
            input_tokens: v.get("input_tokens")?.as_u64()?,
            cached_tokens: v
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_write_tokens: v
                .pointer("/input_tokens_details/cache_write_tokens")
                .and_then(Value::as_u64),
            output_tokens: v.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        };
        if u.cached_tokens > u.input_tokens
            || u.cache_write_tokens
                .is_some_and(|w| w > u.input_tokens - u.cached_tokens)
        {
            return None;
        }
        Some(u)
    }
    pub fn hit_rate(&self) -> Option<f64> {
        (self.input_tokens > 0).then(|| self.cached_tokens as f64 / self.input_tokens as f64)
    }
}

/// Bounded observer. Forwarded bytes never pass through a serializer.
/// A complete SSE terminal event AND clean EOF are needed to commit a lane.
pub struct Observer {
    sse: bool,
    limit: usize,
    buffer: Vec<u8>,
    data: Vec<u8>,
    pub overflow: bool,
    pub terminal: Option<String>,
    pub usage: Option<Usage>,
    capture_output: bool,
    output_items: BTreeMap<usize, Value>,
    output_bytes: usize,
    output_invalid: bool,
}
impl Observer {
    pub fn new(sse: bool, limit: usize) -> Self {
        Self {
            sse,
            limit,
            buffer: Vec::new(),
            data: Vec::new(),
            overflow: false,
            terminal: None,
            usage: None,
            capture_output: false,
            output_items: BTreeMap::new(),
            output_bytes: 0,
            output_invalid: false,
        }
    }
    pub fn capture_output(&mut self) {
        self.capture_output = true;
    }
    fn item(&mut self, index: usize, item: &Value) {
        if !item.is_object() {
            self.output_invalid = true;
            return;
        }
        if let Some(previous) = self.output_items.get(&index) {
            if previous != item {
                self.output_invalid = true;
            }
            return;
        }
        let size = serde_json::to_vec(item).unwrap().len();
        if self.output_bytes.saturating_add(size) > self.limit {
            self.fail_capacity();
            return;
        }
        self.output_bytes += size;
        self.output_items.insert(index, item.clone());
    }
    pub fn output(&self) -> Option<Vec<Value>> {
        if !self.capture_output
            || !self.completed()
            || self.output_invalid
            || self
                .output_items
                .keys()
                .copied()
                .ne(0..self.output_items.len())
        {
            return None;
        }
        Some(self.output_items.values().cloned().collect())
    }
    pub fn feed(&mut self, bytes: &[u8]) {
        if self.overflow {
            return;
        }
        if !self.sse {
            if self.buffer.len().saturating_add(bytes.len()) > self.limit {
                self.fail_capacity();
            } else {
                self.buffer.extend_from_slice(bytes);
            }
            return;
        }
        // Process complete lines without retaining the full stream.
        for segment in bytes.split_inclusive(|b| *b == b'\n') {
            if self
                .buffer
                .len()
                .saturating_add(self.data.len())
                .saturating_add(segment.len())
                > self.limit
            {
                self.fail_capacity();
                return;
            }
            self.buffer.extend_from_slice(segment);
            if segment.last() == Some(&b'\n') {
                let line = std::mem::take(&mut self.buffer);
                let line = line.strip_suffix(b"\n").unwrap_or(&line);
                let line = line.strip_suffix(b"\r").unwrap_or(line);
                if line.is_empty() {
                    self.dispatch();
                } else if let Some(d) = line.strip_prefix(b"data:") {
                    if !self.data.is_empty() {
                        self.data.push(b'\n');
                    }
                    self.data
                        .extend_from_slice(d.strip_prefix(b" ").unwrap_or(d));
                }
            }
        }
    }
    fn fail_capacity(&mut self) {
        self.overflow = true;
        self.buffer.clear();
        self.data.clear();
        self.usage = None;
        self.output_items.clear();
        self.output_invalid = true;
    }
    fn dispatch(&mut self) {
        let data = std::mem::take(&mut self.data);
        if let Ok(v) = serde_json::from_slice::<Value>(&data) {
            match v["type"].as_str().unwrap_or("") {
                "response.output_item.done" if self.capture_output => {
                    if let Some(index) = v["output_index"]
                        .as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                    {
                        self.item(index, &v["item"]);
                    } else {
                        self.output_invalid = true;
                    }
                }
                "response.completed" | "response.failed" | "response.incomplete" => {
                    self.response(&v["response"])
                }
                "error" => self.terminal = Some("error".into()),
                _ => {}
            }
        }
    }
    fn response(&mut self, v: &Value) {
        if self.capture_output {
            if let Some(items) = v["output"].as_array() {
                if !items.is_empty() {
                    if self.output_items.keys().any(|&i| i >= items.len()) {
                        self.output_invalid = true;
                    }
                    for (index, item) in items.iter().enumerate() {
                        self.item(index, item);
                    }
                }
            } else {
                self.output_invalid = true;
            }
        }
        if let Some(status) = v["status"].as_str() {
            self.terminal = Some(status.into());
        }
        self.usage = v.get("usage").and_then(Usage::parse);
    }
    pub fn finish(&mut self) {
        if !self.sse && !self.overflow {
            if let Ok(v) = serde_json::from_slice::<Value>(&self.buffer) {
                self.response(&v);
            }
            self.buffer.clear();
        }
        // An unterminated SSE frame is not a completed event.
    }
    pub fn completed(&self) -> bool {
        !self.overflow
            && self.terminal.as_deref() == Some("completed")
            && (!self.sse || (self.buffer.is_empty() && self.data.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn split_sse_crlf_multiline_utf8_preserves_usage() {
        let wire = b"event: response.completed\r\ndata: {\"type\":\"response.completed\",\r\ndata: \"response\":{\"status\":\"completed\",\"usage\":{\"input_tokens\":100,\"input_tokens_details\":{\"cached_tokens\":80,\"cache_write_tokens\":20}}}}\r\n\r\n";
        for chunk in 1..40 {
            let mut o = Observer::new(true, 4096);
            for c in wire.chunks(chunk) {
                o.feed(c);
            }
            o.finish();
            assert!(o.completed());
            assert_eq!(o.usage.unwrap().hit_rate(), Some(0.8));
        }
    }
    #[test]
    fn incomplete_and_overflow_never_commit() {
        let mut o = Observer::new(true, 100);
        o.feed(b"data: [DONE]\n\n");
        o.finish();
        assert!(!o.completed());
        o.feed(&[b'x'; 101]);
        assert!(o.overflow);
        assert!(!o.completed());
        let mut o = Observer::new(false, 1024);
        o.feed(br#"{"status":"incomplete","usage":{"input_tokens":5}}"#);
        o.finish();
        assert!(!o.completed());
    }
    #[test]
    fn inclusive_tokens_are_never_double_counted() {
        let u = Usage::parse(&json!({"input_tokens":100,"input_tokens_details":{"cached_tokens":80,"cache_write_tokens":20}})).unwrap();
        assert_eq!(u.hit_rate(), Some(0.8));
        assert!(
            Usage::parse(&json!({"input_tokens":100,"input_tokens_details":{"cached_tokens":101}}))
                .is_none()
        );
    }

    #[test]
    fn captured_output_is_bounded_across_items_and_conflicts_are_rejected() {
        fn frame(index: usize, item: Value) -> Vec<u8> {
            format!(
                "data: {}\n\n",
                json!({"type":"response.output_item.done","output_index":index,"item":item})
            )
            .into_bytes()
        }
        let terminal=b"data: {\"type\":\"response.completed\",\"response\":{\"status\":\"completed\",\"output\":[]}}\n\n";
        let mut observer = Observer::new(true, 600);
        observer.capture_output();
        for index in 0..3 {
            observer.feed(&frame(
                index,
                json!({"type":"reasoning","encrypted_content":"A".repeat(240)}),
            ));
        }
        observer.feed(terminal);
        observer.finish();
        assert!(observer.overflow);
        assert!(observer.output().is_none());

        let mut observer = Observer::new(true, 4096);
        observer.capture_output();
        observer.feed(&frame(
            0,
            json!({"type":"compaction","encrypted_content":"first"}),
        ));
        observer.feed(&frame(
            0,
            json!({"type":"compaction","encrypted_content":"conflict"}),
        ));
        observer.feed(terminal);
        observer.finish();
        assert!(observer.output().is_none());
    }

    #[test]
    fn canonical_json_output_and_fragmented_sse_capture_the_same_items() {
        let items = json!([{ "type":"compaction","encrypted_content":"opaque+/=" },
            { "type":"message","role":"assistant","phase":"final_answer","content":"ready" }]);
        let response = json!({"status":"completed","output":items});
        let mut plain = Observer::new(false, 4096);
        plain.capture_output();
        plain.feed(&serde_json::to_vec(&response).unwrap());
        plain.finish();
        let wire = format!(
            "data: {}\r\n\r\n",
            json!({"type":"response.completed","response":response})
        );
        let mut sse = Observer::new(true, 4096);
        sse.capture_output();
        for chunk in wire.as_bytes().chunks(3) {
            sse.feed(chunk);
        }
        sse.finish();
        assert_eq!(plain.output(), sse.output());
        assert_eq!(json!(sse.output().unwrap()), items);
    }
}
