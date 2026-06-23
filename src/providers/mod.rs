pub mod claude;
pub mod openai_compat;
pub mod registry;
pub mod ratelimit;

use std::time::SystemTime;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

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
    fn name(&self) -> &str;
    fn models(&self) -> Vec<String>;
    async fn stream(&self, req: StreamRequest) -> Result<mpsc::Receiver<StreamChunk>>;
}
