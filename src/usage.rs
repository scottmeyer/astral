use serde::{Deserialize, Serialize};
use serde_json::Value;

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
        }
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
    }
    fn dispatch(&mut self) {
        let data = std::mem::take(&mut self.data);
        if let Ok(v) = serde_json::from_slice::<Value>(&data) {
            match v["type"].as_str().unwrap_or("") {
                "response.completed" | "response.failed" | "response.incomplete" => {
                    self.response(&v["response"])
                }
                "error" => self.terminal = Some("error".into()),
                _ => {}
            }
        }
    }
    fn response(&mut self, v: &Value) {
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
}
