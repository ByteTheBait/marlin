use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::mpsc;

use crate::engine::UiUpdate;
use crate::providers::{Message, Provider, StreamRequest};

/// Minimum hours between nightly daemon runs.
const COOLDOWN_SECS: u64 = 20 * 60 * 60;

pub fn spawn(
    marlin_dir: PathBuf,
    provider: Arc<dyn Provider>,
    model: String,
    ui_tx: mpsc::Sender<UiUpdate>,
) {
    tokio::spawn(async move {
        if !should_run(&marlin_dir) {
            return;
        }

        // Let the user settle before background work begins.
        tokio::time::sleep(Duration::from_secs(8)).await;

        let sessions_text = read_recent_sessions(&marlin_dir);
        if sessions_text.trim().is_empty() {
            mark_ran(&marlin_dir);
            return;
        }

        let prompt = format!(
            "You are analyzing recent Marlin coding-assistant sessions.\n\
            Suggest exactly 3 reusable skills that would speed up this user's workflow.\n\n\
            Format each skill as a TOML block the user can drop in ~/.marlin/skills/:\n\
            ```toml\n\
            name = \"skill_name\"\n\
            description = \"one line\"\n\
            triggers = [\"keyword1\", \"keyword2\"]\n\n\
            [run]\n\
            type = \"shell\"  # or \"prompt\"\n\
            command = \"cmd {{query}}\"  # for shell\n\
            # template = \"AI prompt {{input}}\"  # for prompt\n\
            ```\n\n\
            Recent conversation excerpts:\n{sessions_text}"
        );

        let req = StreamRequest {
            model: model.clone(),
            messages: vec![Message::new_user(prompt)],
            system_prompt: String::new(),
            max_tokens: 1200,
            tools: vec![],
            thinking: false,
        };

        let mut result = String::new();
        if let Ok(mut stream) = provider.stream(req).await {
            while let Some(chunk) = stream.recv().await {
                result.push_str(&chunk.content);
                if chunk.done {
                    break;
                }
            }
        }

        if result.trim().is_empty() {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M");
        let content = format!("# Skill Suggestions — {timestamp}\n\n{result}\n");
        let _ = std::fs::write(marlin_dir.join("skill_suggestions.md"), &content);

        mark_ran(&marlin_dir);

        let _ = ui_tx
            .send(UiUpdate::SystemMsg(
                "Nightly skill analysis done — use /skill suggest to view suggestions.".into(),
            ))
            .await;
    });
}

fn should_run(marlin_dir: &Path) -> bool {
    let path = marlin_dir.join("skill_daemon_last.txt");
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(last) = data.trim().parse::<u64>() {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            return now.saturating_sub(last) > COOLDOWN_SECS;
        }
    }
    true
}

fn mark_ran(marlin_dir: &Path) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let _ = std::fs::write(marlin_dir.join("skill_daemon_last.txt"), now.to_string());
}

fn read_recent_sessions(marlin_dir: &Path) -> String {
    let sessions_dir = marlin_dir.join("sessions");
    let mut entries: Vec<_> = std::fs::read_dir(&sessions_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .collect();

    entries.sort_by_key(|e| {
        e.metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    entries.reverse();
    entries.truncate(5);

    let mut text = String::new();
    for entry in entries {
        if let Ok(data) = std::fs::read_to_string(entry.path()) {
            if let Ok(session) = serde_json::from_str::<serde_json::Value>(&data) {
                if let Some(messages) = session.get("messages").and_then(|m| m.as_array()) {
                    for msg in messages.iter().take(30) {
                        if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                            if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                                let snippet: String = content.chars().take(250).collect();
                                text.push_str(&format!("User: {snippet}\n"));
                            }
                        }
                    }
                }
            }
            text.push('\n');
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emoji_prefixed_long_message_does_not_panic() {
        // Regression test: content.chars().take(250) must not slice mid-UTF-8-char
        // the way the old `&content[..content.len().min(250)]` byte-slice did.
        let marlin_dir = std::env::temp_dir().join("marlin_daemon_test_emoji");
        let sessions_dir = marlin_dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        // A multi-byte emoji sitting right at what would be byte offset ~250
        // when repeated — this is exactly the shape that panicked before the fix.
        let long_content = format!("🎉{}", "x".repeat(400));
        let session = serde_json::json!({
            "messages": [
                { "role": "user", "content": long_content },
            ]
        });
        std::fs::write(
            sessions_dir.join("session1.json"),
            serde_json::to_string(&session).unwrap(),
        )
        .unwrap();

        let text = read_recent_sessions(&marlin_dir);
        assert!(text.starts_with("User: 🎉"));

        let _ = std::fs::remove_dir_all(&marlin_dir);
    }
}
