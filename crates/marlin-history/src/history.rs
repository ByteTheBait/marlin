use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use marlin_providers::{Message, ToolCallMsg};

const MAX_INPUT_ENTRIES: usize = 500;

// ── Input history ────────────────────────────────────────────────────────────

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct InputHistory {
    pub entries: Vec<String>,
    #[serde(skip)]
    path: PathBuf,
}

impl InputHistory {
    pub fn load(marlin_dir: &Path) -> Self {
        let path = marlin_dir.join("input_history.json");
        let mut h = Self {
            entries: vec![],
            path: path.clone(),
        };
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Ok(loaded) = serde_json::from_str::<Self>(&data) {
                h.entries = loaded.entries;
            }
        }
        h
    }

    pub fn add(&mut self, entry: &str) {
        if entry.is_empty() {
            return;
        }
        if self.entries.first().map(String::as_str) == Some(entry) {
            return;
        }
        self.entries.insert(0, entry.to_string());
        if self.entries.len() > MAX_INPUT_ENTRIES {
            self.entries.truncate(MAX_INPUT_ENTRIES);
        }
        if let Ok(data) = serde_json::to_string(&self) {
            let _ = std::fs::write(&self.path, data);
        }
    }
}

// ── Chat sessions ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionToolCall {
    pub id: String,
    pub name: String,
    pub input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub tool_calls: Vec<SessionToolCall>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_use_id: String,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEntry {
    pub role: String,
    pub content: String,
    pub time: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_name: String,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub project: String,
    pub work_dir: String,
    pub entries: Vec<SessionEntry>,
    pub messages: Vec<SessionMessage>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Session {
    pub fn new(project: &str, work_dir: &str) -> Self {
        let now = Utc::now();
        Self {
            id: now.format("%Y-%m-%dT%H-%M-%S").to_string(),
            project: project.to_string(),
            work_dir: work_dir.to_string(),
            entries: vec![],
            messages: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    pub fn summary(&self) -> String {
        let mut msg_count = 0usize;
        let mut preview = String::new();
        // Count user/assistant turns. `messages` is the field that's actually
        // persisted (populated from engine history in save_session); `entries`
        // is never filled, so counting it would always show "0 msgs".
        for m in &self.messages {
            if m.role == "user" || m.role == "assistant" {
                msg_count += 1;
            }
            if m.role == "user" && preview.is_empty() {
                preview = m.content.chars().take(60).collect();
                if m.content.len() > 60 {
                    preview.push_str("...");
                }
            }
        }
        format!(
            "{}  [{} msgs]  {}",
            self.updated_at.format("%b %d %H:%M"),
            msg_count,
            preview
        )
    }
}

pub fn sessions_dir(marlin_dir: &Path) -> PathBuf {
    let dir = marlin_dir.join("sessions");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn save_session(marlin_dir: &Path, session: &mut Session) {
    session.updated_at = Utc::now();
    if let Ok(data) = serde_json::to_string_pretty(session) {
        let path = sessions_dir(marlin_dir).join(format!("{}.json", session.id));
        let _ = std::fs::write(path, data);
    }
}

pub fn list_sessions(marlin_dir: &Path) -> Result<Vec<Session>> {
    let dir = sessions_dir(marlin_dir);
    let mut sessions: Vec<Session> = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|data| serde_json::from_str::<Session>(&data).ok())
        .collect();
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

pub fn clear_sessions(marlin_dir: &Path) -> Result<()> {
    let dir = marlin_dir.join("sessions");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(())
}

// Convert between provider Message and session SessionMessage
pub fn to_session_message(m: &Message) -> SessionMessage {
    SessionMessage {
        role: m.role.clone(),
        content: m.content.clone(),
        tool_calls: m
            .tool_calls
            .iter()
            .map(|tc| SessionToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            })
            .collect(),
        tool_call_id: m.tool_call_id.clone(),
        tool_use_id: m.tool_use_id.clone(),
        is_error: m.is_error,
    }
}

pub fn from_session_message(m: &SessionMessage) -> Message {
    Message {
        role: m.role.clone(),
        content: m.content.clone(),
        tool_calls: m
            .tool_calls
            .iter()
            .map(|tc| ToolCallMsg {
                id: tc.id.clone(),
                name: tc.name.clone(),
                input: tc.input.clone(),
            })
            .collect(),
        tool_call_id: m.tool_call_id.clone(),
        tool_use_id: m.tool_use_id.clone(),
        images: vec![],
        is_error: m.is_error,
    }
}
