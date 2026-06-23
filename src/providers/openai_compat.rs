use std::collections::HashMap;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{
    Message, Provider, StreamChunk, StreamRequest, ToolCall, ToolDef,
    ratelimit::{parse_rate_limit_state, retry_after_seconds},
};

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
                "anthropic/claude-sonnet-4-5".into(),
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

        let mut body = serde_json::json!({
            "model": req.model,
            "messages": messages,
            "stream": true,
            "max_tokens": req.max_tokens,
        });
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }

        let url = format!("{}/chat/completions", self.endpoint);
        let mut builder = reqwest::Client::new()
            .post(&url)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body)?);

        if !self.api_key.is_empty() {
            builder = builder.header("authorization", format!("Bearer {}", self.api_key));
        }

        let resp = builder.send().await?;
        let (tx, rx) = mpsc::channel::<StreamChunk>(64);

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
                out.push(serde_json::json!({"role": m.role, "content": m.content}));
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
