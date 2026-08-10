use anyhow::{anyhow, Result};
use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;

use super::{
    Message, Provider, StreamChunk, StreamRequest, ToolCall, ToolDef,
    ratelimit::{parse_rate_limit_state, retry_after_seconds},
};

pub struct ClaudeProvider {
    api_key: String,
}

impl ClaudeProvider {
    pub fn new(api_key: String) -> Self {
        Self { api_key }
    }
}

#[async_trait]
impl Provider for ClaudeProvider {
    fn name(&self) -> &str { "claude" }

    fn models(&self) -> Vec<String> {
        vec![
            "claude-opus-4-8".into(),
            "claude-sonnet-5".into(),
            "claude-haiku-4-5-20251001".into(),
            "claude-3-5-sonnet-20241022".into(),
            "claude-3-5-haiku-20241022".into(),
        ]
    }

    async fn count_tokens(&self, req: &StreamRequest) -> Option<usize> {
        if self.api_key.is_empty() { return None; }

        let messages = marshal_claude_messages(&req.messages);
        let tools = marshal_claude_tools(&req.tools);
        let mut body = serde_json::json!({
            "model": req.model,
            "messages": messages,
        });
        if !req.system_prompt.is_empty() {
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": req.system_prompt,
            }]);
        }
        if !tools.is_empty() {
            body["tools"] = serde_json::json!(tools);
        }

        let resp = super::http_client()
            .post("https://api.anthropic.com/v1/messages/count_tokens")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() { return None; }
        let json: serde_json::Value = resp.json().await.ok()?;
        json["input_tokens"].as_u64().map(|n| n as usize)
    }

    async fn stream(&self, req: StreamRequest) -> Result<mpsc::Receiver<StreamChunk>> {
        if self.api_key.is_empty() {
            return Err(anyhow!("claude: no API key set — use /key claude <key>"));
        }

        let body = build_claude_request(&req);
        let body_bytes = serde_json::to_vec(&body)?;

        let client = super::http_client();
        let resp = client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "prompt-caching-2024-07-31")
            .header("content-type", "application/json")
            .body(body_bytes)
            .send()
            .await?;

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
            let msg = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
                .unwrap_or(text);
            let _ = tx.send(StreamChunk {
                content: String::new(),
                done: false,
                error: Some(anyhow!("claude error {status}: {msg}")),
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
            // Guards against a malicious/misbehaving endpoint sending an
            // unterminated line forever, which would otherwise grow `buf`
            // without bound while we wait for a '\n'.
            const MAX_LINE_BUF: usize = 4 * 1024 * 1024;

            // Pending tool calls being accumulated from the stream
            struct PendingTool {
                id: String,
                name: String,
                input: String,
            }
            let mut pending: Vec<PendingTool> = Vec::new();
            let mut current_tool_idx: Option<usize> = None;

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
                        error: Some(anyhow!("claude: response line exceeded {MAX_LINE_BUF} bytes without a newline, aborting stream")),
                        tool_calls: vec![],
                        retry_after: 0,
                        rate_limit: None,
                    }).await;
                    return;
                }

                // Process complete SSE lines
                loop {
                    match buf.find('\n') {
                        None => break,
                        Some(pos) => {
                            let line = buf[..pos].trim().to_string();
                            buf = buf[pos + 1..].to_string();

                            if !line.starts_with("data: ") {
                                continue;
                            }
                            let data = &line["data: ".len()..];

                            let event: Value = match serde_json::from_str(data) {
                                Ok(v) => v,
                                Err(_) => continue,
                            };

                            match event["type"].as_str().unwrap_or("") {
                                "content_block_start" => {
                                    if event["content_block"]["type"].as_str() == Some("tool_use") {
                                        let id = event["content_block"]["id"]
                                            .as_str().unwrap_or("").to_string();
                                        let name = event["content_block"]["name"]
                                            .as_str().unwrap_or("").to_string();
                                        pending.push(PendingTool { id, name, input: String::new() });
                                        current_tool_idx = Some(pending.len() - 1);
                                    } else {
                                        current_tool_idx = None;
                                    }
                                }
                                "content_block_delta" => {
                                    let delta = &event["delta"];
                                    match delta["type"].as_str().unwrap_or("") {
                                        "text_delta" => {
                                            if let Some(text) = delta["text"].as_str() {
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
                                        }
                                        "input_json_delta" => {
                                            if let (Some(idx), Some(part)) = (
                                                current_tool_idx,
                                                delta["partial_json"].as_str(),
                                            ) {
                                                pending[idx].input.push_str(part);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                                "content_block_stop" => {
                                    current_tool_idx = None;
                                }
                                "message_stop" => {
                                    let calls: Vec<ToolCall> = pending.drain(..).map(|pt| ToolCall {
                                        id: pt.id,
                                        name: pt.name,
                                        input: if pt.input.is_empty() {
                                            "{}".into()
                                        } else {
                                            pt.input
                                        },
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
                                _ => {}
                            }
                        }
                    }
                }
            }

            let calls: Vec<ToolCall> = pending.drain(..).map(|pt| ToolCall {
                id: pt.id,
                name: pt.name,
                input: if pt.input.is_empty() { "{}".into() } else { pt.input },
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

fn build_claude_request(req: &StreamRequest) -> Value {
    let messages = marshal_claude_messages(&req.messages);
    let tools = marshal_claude_tools(&req.tools);

    let system: Value = if req.system_prompt.is_empty() {
        Value::Null
    } else {
        serde_json::json!([{
            "type": "text",
            "text": req.system_prompt,
            "cache_control": { "type": "ephemeral" }
        }])
    };

    let mut body = serde_json::json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens,
        "stream": true,
    });

    if !system.is_null() {
        body["system"] = system;
    }
    if !tools.is_empty() {
        body["tools"] = serde_json::json!(tools);
    }

    body
}

fn marshal_claude_messages(msgs: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for m in msgs {
        match m.role.as_str() {
            "assistant" if !m.tool_calls.is_empty() => {
                let mut content: Vec<Value> = Vec::new();
                if !m.content.is_empty() {
                    content.push(serde_json::json!({"type": "text", "text": m.content}));
                }
                for tc in &m.tool_calls {
                    let input: Value = serde_json::from_str(&tc.input)
                        .unwrap_or(serde_json::json!({}));
                    content.push(serde_json::json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name,
                        "input": input,
                    }));
                }
                out.push(serde_json::json!({"role": "assistant", "content": content}));
            }
            "tool" => {
                out.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": m.tool_use_id,
                        "content": m.content,
                        "is_error": m.is_error,
                    }]
                }));
            }
            _ => {
                out.push(serde_json::json!({"role": m.role, "content": m.content}));
            }
        }
    }
    out
}

fn marshal_claude_tools(defs: &[ToolDef]) -> Vec<Value> {
    defs.iter().map(|d| {
        let mut props: serde_json::Map<String, Value> = serde_json::Map::new();
        for p in &d.properties {
            props.insert(p.name.clone(), serde_json::json!({
                "type": p.ty,
                "description": p.description,
            }));
        }
        serde_json::json!({
            "name": d.name,
            "description": d.description,
            "input_schema": {
                "type": "object",
                "properties": props,
                "required": d.required,
            }
        })
    }).collect()
}
