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
    ToolResult {
        is_error: bool,
    },
    /// The final summary text from a `mark_complete` tool call — rendered in a
    /// muted/grayed-out style rather than as a normal assistant message.
    Summary,
    /// A steering command/note the user sent while the model was working —
    /// rendered as a distinct text field in the model's output area without
    /// interrupting the in-flight stream.
    Steer,
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
        Self {
            role: EntryRole::System,
            content: content.into(),
            tool_name: String::new(),
            time: Local::now(),
        }
    }
    pub(super) fn error(content: &str) -> Self {
        Self {
            role: EntryRole::Error,
            content: content.into(),
            tool_name: String::new(),
            time: Local::now(),
        }
    }
}
