use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── Command types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CommandKind {
    Shell,
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandRun {
    #[serde(rename = "type")]
    pub kind: CommandKind,
    /// Shell command to run. Use {args} (bare, unquoted) for text after the slash
    /// command — it is substituted as a single shell-quoted word.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    /// Prompt template for prompt-type commands. Use {input} for the args.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserCommand {
    /// The slash command name (without the leading /). Must be a single word.
    pub name: String,
    pub description: String,
    /// Argument hint shown in autocomplete (e.g. "[--flag]" or "<query>").
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub args: String,
    pub run: CommandRun,
}

/// Lightweight version sent to the TUI for autocomplete suggestions.
#[derive(Debug, Clone)]
pub struct UserCommandDef {
    pub name: String,
    pub description: String,
    pub args: String,
}

impl From<&UserCommand> for UserCommandDef {
    fn from(c: &UserCommand) -> Self {
        Self {
            name: c.name.clone(),
            description: c.description.clone(),
            args: c.args.clone(),
        }
    }
}

// ── I/O ──────────────────────────────────────────────────────────────────────

pub fn commands_dir(marlin_dir: &Path) -> PathBuf {
    let d = marlin_dir.join("commands");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load_all(marlin_dir: &Path) -> Vec<UserCommand> {
    let dir = commands_dir(marlin_dir);
    let mut commands = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return commands };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(data) = std::fs::read_to_string(&path) {
            match toml::from_str::<UserCommand>(&data) {
                Ok(cmd) => commands.push(cmd),
                Err(e) => eprintln!("commands: parse error in {:?}: {e}", path.file_name()),
            }
        }
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    commands
}

pub fn save_command(marlin_dir: &Path, cmd: &UserCommand) -> Result<PathBuf> {
    let dir = commands_dir(marlin_dir);
    let filename = format!("{}.toml", cmd.name.replace([' ', '/'], "_").to_lowercase());
    let path = dir.join(&filename);
    let data = toml::to_string_pretty(cmd)?;
    std::fs::write(&path, data)?;
    Ok(path)
}

// ── Default commands ──────────────────────────────────────────────────────────

/// Install one working example command on first run — same write-if-missing
/// pattern as `skills::install_defaults`. Never overwrites a user's file.
pub fn install_defaults(marlin_dir: &Path) {
    let dir = commands_dir(marlin_dir);
    for cmd in default_commands() {
        let path = dir.join(format!("{}.toml", cmd.name));
        if !path.exists() {
            if let Ok(data) = toml::to_string_pretty(&cmd) {
                let _ = std::fs::write(path, data);
            }
        }
    }
}

fn default_commands() -> Vec<UserCommand> {
    vec![UserCommand {
        name: "status".into(),
        description: "Quick git status for the working directory".into(),
        args: String::new(),
        run: CommandRun {
            kind: CommandKind::Shell,
            command: "git status --short".into(),
            template: String::new(),
        },
    }]
}
