use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{
    Message, Provider, StreamChunk, StreamRequest, ToolCall, ToolDef,
    ratelimit::{parse_rate_limit_state, retry_after_seconds},
};

/// Groq (and some other OpenAI-compatible providers) reject a request outright
/// if the *requested* `max_tokens` alone would exceed the account's
/// tokens-per-minute cap for that model — even when the actual prompt is
/// trivial (e.g. "hi"). The cap is per org/tier and not something we can know
/// ahead of time, so we learn it from the first 413 and cache it per
/// provider+model for the rest of the process, instead of eating a failed
/// round-trip on every single message.
static LEARNED_MAX_TOKENS: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();

fn learned_cache() -> &'static Mutex<HashMap<String, usize>> {
    LEARNED_MAX_TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

const TPM_SAFETY_MARGIN: usize = 200;
const MIN_MAX_TOKENS: usize = 256;

/// Rough size of everything going out over the wire except `max_tokens`
/// itself — `messages` already includes the marshaled system prompt.
fn estimate_request_tokens(messages: &[Value], tools: &[Value]) -> usize {
    let msg_chars: usize = messages.iter().map(|m| m.to_string().len()).sum();
    let tool_chars: usize = tools.iter().map(|t| t.to_string().len()).sum();
    (msg_chars + tool_chars).saturating_add(3) / 4
}

/// Parses Groq's `"... on tokens per minute (TPM): Limit 6000, Used 0, ..."`
/// style 413 body. Returns None for unrelated 413s (e.g. oversized payloads)
/// so we don't misapply a max_tokens retry to something it can't fix.
fn parse_tpm_limit(body: &str) -> Option<usize> {
    if !body.contains("tokens per minute") && !body.contains("TPM") {
        return None;
    }
    let idx = body.find("Limit ")?;
    let rest = &body[idx + "Limit ".len()..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

pub struct OpenAiCompatProvider {
    provider_name: String,
    endpoint: String,
    api_key: String,
    model_list: Vec<String>,
}

impl OpenAiCompatProvider {
    pub fn new_ollama(endpoint: &str, model: &str) -> Self {
        Self {
            provider_name: "ollama".into(),
            endpoint: if endpoint.is_empty() { "http://localhost:11434" } else { endpoint }.into(),
            api_key: String::new(),
            model_list: vec![model.to_string()],
        }
    }

    pub fn new_fireworks(api_key: &str, endpoint: &str) -> Self {
        Self {
            provider_name: "fireworks".into(),
            endpoint: if endpoint.is_empty() {
                "https://api.fireworks.ai/inference/v1"
            } else { endpoint }.into(),
            api_key: api_key.into(),
            model_list: vec![
                "accounts/fireworks/models/llama-v3p1-70b-instruct".into(),
                "accounts/fireworks/models/llama-v3p1-8b-instruct".into(),
                "accounts/fireworks/models/deepseek-coder-v2-instruct".into(),
            ],
        }
    }

    pub fn new_groq(api_key: &str, endpoint: &str) -> Self {
        Self {
            provider_name: "groq".into(),
            endpoint: if endpoint.is_empty() {
                "https://api.groq.com/openai/v1"
            } else { endpoint }.into(),
            api_key: api_key.into(),
            model_list: vec![
                "llama-3.3-70b-versatile".into(),
                "llama-3.1-8b-instant".into(),
                "llama3-70b-8192".into(),
                "mixtral-8x7b-32768".into(),
            ],
        }
    }

    pub fn new_moonshot(api_key: &str, endpoint: &str) -> Self {
        Self {
            provider_name: "moonshot".into(),
            endpoint: if endpoint.is_empty() {
                "https://api.moonshot.cn/v1"
            } else { endpoint }.into(),
            api_key: api_key.into(),
            model_list: vec![
                "moonshot-v1-8k".into(),
                "moonshot-v1-32k".into(),
                "moonshot-v1-128k".into(),
            ],
        }
    }

    pub fn new_openrouter(api_key: &str, endpoint: &str) -> Self {
        Self {
            provider_name: "openrouter".into(),
            endpoint: if endpoint.is_empty() {
                "https://openrouter.ai/api/v1"
            } else { endpoint }.into(),
            api_key: api_key.into(),
            model_list: vec![
                "anthropic/claude-sonnet-5".into(),
                "openai/gpt-4o".into(),
                "google/gemini-2.0-flash-001".into(),
                "meta-llama/llama-4-maverick".into(),
                "deepseek/deepseek-r1".into(),
                "mistralai/mistral-large".into(),
            ],
        }
    }

    pub fn new_custom(api_key: &str, endpoint: &str) -> Self {
        Self {
            provider_name: "custom".into(),
            endpoint: if endpoint.is_empty() {
                "http://localhost:8080/v1"
            } else { endpoint }.into(),
            api_key: api_key.into(),
            model_list: vec!["default".into()],
        }
    }

    /// Generic constructor for user-defined providers from ~/.marlin/providers/*.toml.
    pub fn new_generic(name: &str, endpoint: &str, api_key: &str, models: Vec<String>) -> Self {
        Self {
            provider_name: name.into(),
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model_list: if models.is_empty() { vec!["default".into()] } else { models },
        }
    }
}

#[async_trait]
impl Provider for OpenAiCompatProvider {
    fn name(&self) -> &str { &self.provider_name }

    fn models(&self) -> Vec<String> { self.model_list.clone() }

    async fn stream(&self, req: StreamRequest) -> Result<mpsc::Receiver<StreamChunk>> {
        if self.api_key.is_empty() && self.provider_name != "ollama" && self.provider_name != "custom" {
            return Err(anyhow!(
                "{}: no API key set — use /key {} <key>",
                self.provider_name, self.provider_name
            ));
        }

        let messages = marshal_openai_messages(&req.messages, &req.system_prompt);
        let tools = marshal_openai_tools(&req.tools);

        let cache_key = format!("{}/{}", self.provider_name, req.model);
        let mut max_tokens = learned_cache().lock().unwrap()
            .get(&cache_key)
            .map(|&learned| req.max_tokens.min(learned))
            .unwrap_or(req.max_tokens);

        let url = format!("{}/chat/completions", self.endpoint);
        let client = super::http_client();

        let mut body = build_openai_body(&req.model, &messages, &tools, max_tokens);
        let mut resp = send_openai_request(&client, &url, &self.api_key, &body).await?;
        let (tx, rx) = mpsc::channel::<StreamChunk>(64);

        // A 413 with a TPM limit means the *requested* max_tokens alone blew
        // the account's per-minute budget for this model — retry once with a
        // budget that actually fits, and remember it so future turns don't
        // pay for this round-trip again.
        if resp.status().as_u16() == 413 {
            let text = resp.text().await.unwrap_or_default();
            let retry_max = parse_tpm_limit(&text).map(|limit| {
                let prompt_estimate = estimate_request_tokens(&messages, &tools);
                limit.saturating_sub(prompt_estimate).saturating_sub(TPM_SAFETY_MARGIN).max(MIN_MAX_TOKENS)
            });
            match retry_max {
                Some(safe_max) if safe_max < max_tokens => {
                    max_tokens = safe_max;
                    learned_cache().lock().unwrap().insert(cache_key, safe_max);
                    body = build_openai_body(&req.model, &messages, &tools, max_tokens);
                    resp = send_openai_request(&client, &url, &self.api_key, &body).await?;
                }
                _ => {
                    let name = self.provider_name.clone();
                    let _ = tx.send(StreamChunk {
                        content: String::new(),
                        done: false,
                        error: Some(anyhow!("{name} error 413: {text}")),
                        tool_calls: vec![],
                        retry_after: 0,
                        rate_limit: None,
                    }).await;
                    return Ok(rx);
                }
            }
        }

        if resp.status().as_u16() == 429 {
            let secs = retry_after_seconds(resp.headers(), 60);
            let _ = tx.send(StreamChunk {
                content: String::new(),
                done: false,
                error: None,
                tool_calls: vec![],
                retry_after: secs,
                rate_limit: None,
            }).await;
            return Ok(rx);
        }

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let name = self.provider_name.clone();
            let _ = tx.send(StreamChunk {
                content: String::new(),
                done: false,
                error: Some(anyhow!("{name} error {status}: {text}")),
                tool_calls: vec![],
                retry_after: 0,
                rate_limit: None,
            }).await;
            return Ok(rx);
        }

        let rl = parse_rate_limit_state(resp.headers());
        let mut stream = resp.bytes_stream();

        tokio::spawn(async move {
            let mut buf = String::new();
            // See the matching comment in claude.rs — bounds an unterminated
            // line from growing `buf` without limit.
            const MAX_LINE_BUF: usize = 4 * 1024 * 1024;

            struct AccTool {
                id: String,
                name: String,
                args: String,
            }
            let mut acc: HashMap<usize, AccTool> = HashMap::new();

            while let Some(chunk) = stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(StreamChunk {
                            content: String::new(),
                            done: false,
                            error: Some(anyhow!("{e}")),
                            tool_calls: vec![],
                            retry_after: 0,
                            rate_limit: None,
                        }).await;
                        return;
                    }
                };

                buf.push_str(&String::from_utf8_lossy(&chunk));

                if buf.len() > MAX_LINE_BUF {
                    let _ = tx.send(StreamChunk {
                        content: String::new(),
                        done: false,
                        error: Some(anyhow!("response line exceeded {MAX_LINE_BUF} bytes without a newline, aborting stream")),
                        tool_calls: vec![],
                        retry_after: 0,
                        rate_limit: None,
                    }).await;
                    return;
                }

                loop {
                    match buf.find('\n') {
                        None => break,
                        Some(pos) => {
                            let line = buf[..pos].trim().to_string();
                            buf = buf[pos + 1..].to_string();

                            if !line.starts_with("data: ") { continue; }
                            let data = &line["data: ".len()..];
                            if data == "[DONE]" {
                                let mut indices: Vec<usize> = acc.keys().cloned().collect();
                                indices.sort();
                                let calls: Vec<ToolCall> = indices.into_iter().filter_map(|i| {
                                    acc.remove(&i).map(|a| ToolCall {
                                        id: a.id,
                                        name: a.name,
                                        input: if a.args.is_empty() { "{}".into() } else { a.args },
                                    })
                                }).collect();
                                let _ = tx.send(StreamChunk {
                                    content: String::new(),
                                    done: true,
                                    error: None,
                                    tool_calls: calls,
                                    retry_after: 0,
                                    rate_limit: rl,
                                }).await;
                                return;
                            }

                            let event: Value = match serde_json::from_str(data) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };

                            if let Some(choices) = event["choices"].as_array() {
                                for choice in choices {
                                    let delta = &choice["delta"];

                                    if let Some(text) = delta["content"].as_str() {
                                        if !text.is_empty() {
                                            let _ = tx.send(StreamChunk {
                                                content: text.to_string(),
                                                done: false,
                                                error: None,
                                                tool_calls: vec![],
                                                retry_after: 0,
                                                rate_limit: None,
                                            }).await;
                                        }
                                    }

                                    if let Some(tcs) = delta["tool_calls"].as_array() {
                                        for tc in tcs {
                                            let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                                            let entry = acc.entry(idx).or_insert(AccTool {
                                                id: String::new(),
                                                name: String::new(),
                                                args: String::new(),
                                            });
                                            if let Some(id) = tc["id"].as_str() {
                                                if !id.is_empty() { entry.id = id.to_string(); }
                                            }
                                            if let Some(name) = tc["function"]["name"].as_str() {
                                                if !name.is_empty() { entry.name = name.to_string(); }
                                            }
                                            if let Some(args) = tc["function"]["arguments"].as_str() {
                                                entry.args.push_str(args);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let mut indices: Vec<usize> = acc.keys().cloned().collect();
            indices.sort();
            let calls: Vec<ToolCall> = indices.into_iter().filter_map(|i| {
                acc.remove(&i).map(|a| ToolCall {
                    id: a.id,
                    name: a.name,
                    input: if a.args.is_empty() { "{}".into() } else { a.args },
                })
            }).collect();
            let _ = tx.send(StreamChunk {
                content: String::new(),
                done: true,
                error: None,
                tool_calls: calls,
                retry_after: 0,
                rate_limit: rl,
            }).await;
        });

        Ok(rx)
    }
}

fn build_openai_body(model: &str, messages: &[Value], tools: &[Value], max_tokens: usize) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "stream": true,
        "max_tokens": max_tokens,
    });
    if !tools.is_empty() {
        body["tools"] = serde_json::json!(tools);
    }
    body
}

async fn send_openai_request(
    client: &reqwest::Client,
    url: &str,
    api_key: &str,
    body: &Value,
) -> Result<reqwest::Response> {
    let mut builder = client.post(url).header("content-type", "application/json");
    if !api_key.is_empty() {
        builder = builder.header("authorization", format!("Bearer {api_key}"));
    }
    Ok(builder.body(serde_json::to_vec(body)?).send().await?)
}

fn marshal_openai_messages(messages: &[Message], system_prompt: &str) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if !system_prompt.is_empty() {
        out.push(serde_json::json!({"role": "system", "content": system_prompt}));
    }
    for m in messages {
        match m.role.as_str() {
            "assistant" if !m.tool_calls.is_empty() => {
                let calls: Vec<Value> = m.tool_calls.iter().map(|tc| serde_json::json!({
                    "id": tc.id,
                    "type": "function",
                    "function": { "name": tc.name, "arguments": tc.input },
                })).collect();
                let mut msg = serde_json::json!({"role": "assistant", "tool_calls": calls});
                if !m.content.is_empty() {
                    msg["content"] = serde_json::json!(m.content);
                }
                out.push(msg);
            }
            "tool" => {
                out.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": m.tool_call_id,
                    "content": m.content,
                }));
            }
            _ => {
                // Multimodal user message: image_url blocks + text.
                if !m.images.is_empty() {
                    let mut content: Vec<Value> = Vec::new();
                    for (mime, b64) in &m.images {
                        content.push(serde_json::json!({
                            "type": "image_url",
                            "image_url": {
                                "url": format!("data:{mime};base64,{b64}"),
                            }
                        }));
                    }
                    if !m.content.is_empty() {
                        content.push(serde_json::json!({"type": "text", "text": m.content}));
                    }
                    out.push(serde_json::json!({"role": m.role, "content": content}));
                } else {
                    out.push(serde_json::json!({"role": m.role, "content": m.content}));
                }
            }
        }
    }
    out
}

fn marshal_openai_tools(defs: &[ToolDef]) -> Vec<Value> {
    defs.iter().map(|d| {
        let mut props: serde_json::Map<String, Value> = serde_json::Map::new();
        for p in &d.properties {
            props.insert(p.name.clone(), serde_json::json!({
                "type": p.ty,
                "description": p.description,
            }));
        }
        serde_json::json!({
            "type": "function",
            "function": {
                "name": d.name,
                "description": d.description,
                "parameters": {
                    "type": "object",
                    "properties": props,
                    "required": d.required,
                }
            }
        })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_groq_tpm_limit_from_413_body() {
        let body = r#"{"error":{"message":"Request too large for model `llama-3.1-8b-instant` in organization `org_01kf9g03d4f5y88y3cjpcapc7t` service tier `on_demand` on tokens per minute (TPM): Limit 6000, Used 0, Requested 8192. Please reduce your message size and try again.","type":"tokens","code":"rate_limit_exceeded"}}"#;
        assert_eq!(parse_tpm_limit(body), Some(6000));
    }

    #[test]
    fn ignores_413_bodies_unrelated_to_tpm() {
        let body = r#"{"error":{"message":"Request body too large","type":"invalid_request_error"}}"#;
        assert_eq!(parse_tpm_limit(body), None);
    }

    #[test]
    fn retry_budget_fits_under_the_learned_limit() {
        // Mirrors the reported bug: a bare "hi" plus base tool defs, against
        // Groq's 6000 TPM cap for llama-3.1-8b-instant with max_tokens=8192.
        let messages = vec![
            serde_json::json!({"role": "system", "content": "You are Marlin...".repeat(20)}),
            serde_json::json!({"role": "user", "content": "hi"}),
        ];
        let tools: Vec<Value> = vec![];
        let limit = 6000usize;
        let prompt_estimate = estimate_request_tokens(&messages, &tools);
        let safe_max = limit.saturating_sub(prompt_estimate).saturating_sub(TPM_SAFETY_MARGIN).max(MIN_MAX_TOKENS);
        assert!(safe_max < 8192, "retry budget should be below the original request");
        assert!(prompt_estimate + safe_max + TPM_SAFETY_MARGIN <= limit);
    }
}
