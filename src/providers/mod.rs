pub mod claude;
pub mod openai_compat;
pub mod registry;
pub mod ratelimit;
pub mod user_providers;

use std::time::{Duration, SystemTime};

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Shared HTTP client for provider API calls.
///
/// `connect_timeout` bounds the TCP/TLS handshake; `read_timeout` fires only
/// when a read stalls (it resets on every chunk received), so a long but
/// actively-streaming completion is never cut short by it. Neither uses the
/// blanket `timeout()`, which caps the *entire* request including body read
/// and would kill legitimate long streaming responses.
pub fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default()
}

// ── wire types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallMsg>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_use_id: String,   // Claude tool result
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,  // OpenAI tool result
    #[serde(default)]
    pub is_error: bool,
    /// Inline image attachments for this message: (mime_type, base64 data).
    /// Rendered as multimodal content blocks by the providers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<(String, String)>,
}

impl Message {
    pub fn new_user(content: impl Into<String>) -> Self {
        Message {
            role: "user".into(),
            content: content.into(),
            tool_calls: vec![],
            tool_use_id: String::new(),
            tool_call_id: String::new(),
            is_error: false,
            images: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallMsg {
    pub id: String,
    pub name: String,
    pub input: String, // JSON-encoded
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub properties: Vec<ToolProp>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ToolProp {
    pub name: String,
    pub ty: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct RateLimitState {
    pub remaining_requests: i64,  // -1 = unknown
    pub remaining_tokens: i64,    // -1 = unknown
    pub reset_requests_at: Option<SystemTime>,
    pub reset_tokens_at: Option<SystemTime>,
}

#[derive(Debug)]
pub struct StreamChunk {
    pub content: String,
    pub done: bool,
    pub error: Option<anyhow::Error>,
    pub tool_calls: Vec<ToolCall>,
    pub retry_after: u32,  // 0 = not rate-limited
    pub rate_limit: Option<RateLimitState>,
}

#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub system_prompt: String,
    pub max_tokens: usize,
    pub tools: Vec<ToolDef>,
}

// ── provider trait ───────────────────────────────────────────────────────────

#[async_trait]
pub trait Provider: Send + Sync {
    #[allow(dead_code)]
    fn name(&self) -> &str;
    fn models(&self) -> Vec<String>;
    async fn stream(&self, req: StreamRequest) -> Result<mpsc::Receiver<StreamChunk>>;

    /// Exact input token count for the given request.
    /// Returns None if the provider doesn't support this (falls back to heuristic).
    async fn count_tokens(&self, _req: &StreamRequest) -> Option<usize> { None }
}
