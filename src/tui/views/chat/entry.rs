use chrono::{DateTime, Local};

// ── Chat entry ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum EntryRole {
    User,
    Assistant,
    System,
    Error,
    #[allow(dead_code)]
    Output,
    ToolCall,
    ToolResult { is_error: bool },
}

#[derive(Clone)]
pub struct ChatEntry {
    pub role: EntryRole,
    pub content: String,
    pub tool_name: String,
    pub time: DateTime<Local>,
}

impl ChatEntry {
    pub(super) fn system(content: &str) -> Self {
        Self { role: EntryRole::System, content: content.into(), tool_name: String::new(), time: Local::now() }
    }
    pub(super) fn error(content: &str) -> Self {
        Self { role: EntryRole::Error, content: content.into(), tool_name: String::new(), time: Local::now() }
    }
}
