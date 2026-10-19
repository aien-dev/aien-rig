//! Talking to an OpenAI-style chat completions server.
//!
//! Works with the Rig engine, AIEN's `aien-serve-http`, llama.cpp's server,
//! Ollama's `/v1`, LM Studio and anything else that speaks
//! `POST {base}/chat/completions`. Plain text only: no tool calling.

use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{Error, Result};

/// The Rig engine's default address.
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:18120/v1";

/// One chat request: a system message and a user message.
#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
    pub max_tokens: u32,
    pub temperature: f32,
}

/// The model's reply and what the server reported about it.
#[derive(Debug, Clone, Default)]
pub struct ChatReply {
    pub text: String,
    pub finish_reason: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    /// Wall time for the request, including any waiting while the server was busy.
    pub seconds: f64,
    /// How many times the server said it was busy (503/429) before answering.
    pub busy_retries: u32,
}

/// Anything that can answer a chat request. The HTTP client implements it;
/// tests use a fake.
pub trait ChatModel {
    fn chat(&self, req: &ChatRequest) -> Result<ChatReply>;
    /// The model id, for the record.
    fn model_id(&self) -> String;
}

/// A chat model reached over HTTP.
pub struct HttpModel {
    base: String,
    model: String,
    api_key: Option<String>,
    agent: ureq::Agent,
    /// Longest total time to keep retrying while the server is busy.
    pub max_busy_wait: Duration,
}

/// Add `/v1` when the user gave only `http://host:port`.
pub fn normalize_base(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    let u = u.strip_suffix("/chat/completions").unwrap_or(u);
    let after_scheme = u.split_once("://").map(|x| x.1).unwrap_or(u);
    let with_scheme = if u.contains("://") { u.to_string() } else { format!("http://{u}") };
    if after_scheme.contains('/') {
        with_scheme
    } else {
        format!("{with_scheme}/v1")
    }
}

impl HttpModel {
    /// Connect to `base_url`. If `model` is `None`, the first model the server
    /// lists at `/models` is used.
    pub fn connect(
        base_url: &str,
        model: Option<&str>,
        api_key: Option<&str>,
        request_timeout: Duration,
    ) -> Result<HttpModel> {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(10))
            .timeout_read(request_timeout)
            .timeout_write(Duration::from_secs(60))
            .build();
        let mut m = HttpModel {
            base: normalize_base(base_url),
            model: model.unwrap_or_default().to_string(),
            api_key: api_key.filter(|k| !k.is_empty()).map(str::to_string),
            agent,
            max_busy_wait: Duration::from_secs(900),
        };
        if m.model.is_empty() {
            let models = m.list_models()?;
            m.model = models.into_iter().next().ok_or_else(|| {
                Error::Server(format!("{} lists no models; pass --model", m.base))
            })?;
        }
        Ok(m)
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// Model ids from `GET {base}/models`.
    pub fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/models", self.base);
        let mut req = self.agent.get(&url);
        if let Some(k) = &self.api_key {
            req = req.set("Authorization", &format!("Bearer {k}"));
        }
        let resp = req.call().map_err(|e| self.describe(e))?;
        let body = resp.into_string().map_err(|e| Error::Server(format!("reading {url}: {e}")))?;
        let v: Value = serde_json::from_str(&body)
            .map_err(|e| Error::Server(format!("{url} did not return JSON: {e}")))?;
        Ok(v.get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    fn describe(&self, e: ureq::Error) -> Error {
        match e {
            ureq::Error::Status(code, resp) => {
                let body = resp.into_string().unwrap_or_default();
                let body: String = body.chars().take(300).collect();
                Error::Server(format!("{} answered HTTP {code}: {body}", self.base))
            }
            ureq::Error::Transport(t) => Error::Server(format!(
                "cannot reach the model server at {} ({t}). Is it running?",
                self.base
            )),
        }
    }
}

impl ChatModel for HttpModel {
    fn model_id(&self) -> String {
        self.model.clone()
    }

    fn chat(&self, req: &ChatRequest) -> Result<ChatReply> {
        let url = format!("{}/chat/completions", self.base);
        let body = json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": req.system},
                {"role": "user", "content": req.user},
            ],
            "max_tokens": req.max_tokens,
            "temperature": req.temperature,
            "stream": false,
        })
        .to_string();
        let started = Instant::now();
        let mut backoff = 2.0f64;
        let mut transport_failures = 0u32;
        let mut busy_retries = 0u32;
        loop {
            let mut r = self.agent.post(&url).set("Content-Type", "application/json");
            if let Some(k) = &self.api_key {
                r = r.set("Authorization", &format!("Bearer {k}"));
            }
            match r.send_string(&body) {
                Ok(resp) => {
                    let text = resp
                        .into_string()
                        .map_err(|e| Error::Server(format!("reading reply: {e}")))?;
                    let mut reply = parse_reply(&text)?;
                    reply.seconds = started.elapsed().as_secs_f64();
                    reply.busy_retries = busy_retries;
                    return Ok(reply);
                }
                Err(ureq::Error::Status(code, resp)) if code == 503 || code == 429 => {
                    let wait = resp
                        .header("Retry-After")
                        .and_then(|v| v.trim().parse::<f64>().ok())
                        .unwrap_or(backoff)
                        .clamp(1.0, 30.0);
                    if started.elapsed() + Duration::from_secs_f64(wait) > self.max_busy_wait {
                        return Err(Error::Server(format!(
                            "{} stayed busy for {} s; try again when it is free",
                            self.base,
                            started.elapsed().as_secs()
                        )));
                    }
                    busy_retries += 1;
                    sleep(Duration::from_secs_f64(wait));
                    backoff = (backoff * 2.0).min(30.0);
                }
                Err(ureq::Error::Transport(t)) if transport_failures < 2 => {
                    // One or two quick retries for a dropped connection.
                    let _ = t;
                    transport_failures += 1;
                    sleep(Duration::from_secs(2 * transport_failures as u64));
                }
                Err(e) => return Err(self.describe(e)),
            }
        }
    }
}

/// Parse a chat completions response body.
pub fn parse_reply(body: &str) -> Result<ChatReply> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| Error::Server(format!("reply was not JSON: {e}")))?;
    let choice = v
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| Error::Server("reply had no choices".into()))?;
    let text = choice
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .or_else(|| choice.get("text").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let usage = v.get("usage");
    Ok(ChatReply {
        text,
        finish_reason: choice.get("finish_reason").and_then(Value::as_str).map(str::to_string),
        prompt_tokens: usage.and_then(|u| u.get("prompt_tokens")).and_then(Value::as_u64),
        completion_tokens: usage.and_then(|u| u.get("completion_tokens")).and_then(Value::as_u64),
        seconds: 0.0,
        busy_retries: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_urls() {
        assert_eq!(normalize_base("http://127.0.0.1:18120"), "http://127.0.0.1:18120/v1");
        assert_eq!(normalize_base("http://127.0.0.1:18120/v1/"), "http://127.0.0.1:18120/v1");
        assert_eq!(normalize_base("127.0.0.1:8080"), "http://127.0.0.1:8080/v1");
        assert_eq!(
            normalize_base("http://h:1/v1/chat/completions"),
            "http://h:1/v1"
        );
        assert_eq!(normalize_base("https://x.example/api/v1"), "https://x.example/api/v1");
    }

    #[test]
    fn reply_parsing() {
        let r = parse_reply(r#"{"choices":[{"message":{"role":"assistant","content":"hi"},"finish_reason":"length"}],"usage":{"prompt_tokens":5,"completion_tokens":7}}"#).unwrap();
        assert_eq!(r.text, "hi");
        assert_eq!(r.finish_reason.as_deref(), Some("length"));
        assert_eq!(r.completion_tokens, Some(7));
        assert!(parse_reply("{}").is_err());
        assert!(parse_reply("nope").is_err());
    }
}
