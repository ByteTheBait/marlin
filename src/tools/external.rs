use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::providers::{ToolDef, ToolProp};

// ── External tool definition ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalToolProp {
    pub name: String,
    /// JSON schema type shown to the LLM — almost always "string".
    #[serde(rename = "type", default = "default_prop_type")]
    pub ty: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub required: bool,
}

fn default_prop_type() -> String { "string".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalToolRun {
    /// Only "shell" is supported. Property values are substituted as {name}.
    #[serde(rename = "type")]
    pub kind: String,
    /// Shell command. Use `{property_name}` as placeholders for LLM-supplied values.
    pub command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalTool {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub properties: Vec<ExternalToolProp>,
    pub run: ExternalToolRun,
}

impl ExternalTool {
    /// Convert to a ToolDef for inclusion in the LLM's tool list.
    pub fn to_tool_def(&self) -> ToolDef {
        ToolDef {
            name: self.name.clone(),
            description: self.description.clone(),
            properties: self.properties.iter().map(|p| ToolProp {
                name: p.name.clone(),
                ty: p.ty.clone(),
                description: p.description.clone(),
            }).collect(),
            required: self.properties.iter()
                .filter(|p| p.required)
                .map(|p| p.name.clone())
                .collect(),
        }
    }

    /// Execute the tool's shell command with the provided property values.
    /// Returns `(output, is_error)`.
    pub fn run(&self, input: &HashMap<String, String>, work_dir: &str, clean_env: bool) -> (String, bool) {
        // Substitute {name} placeholders from the LLM's input.
        let mut cmd = self.run.command.clone();
        for (k, v) in input {
            cmd = cmd.replace(&format!("{{{k}}}"), v);
        }
        // Remove any remaining {placeholder} — optional props not supplied by the LLM.
        let mut cleaned = String::with_capacity(cmd.len());
        let mut in_brace = false;
        for c in cmd.chars() {
            match c {
                '{' => { in_brace = true; }
                '}' => { in_brace = false; }
                _ if !in_brace => cleaned.push(c),
                _ => {}
            }
        }

        use std::process::Command;
        let mut command = Command::new("sh");
        command.arg("-c").arg(&cleaned).current_dir(work_dir);
        if clean_env {
            command.env_clear();
            for var in &["PATH", "HOME", "USER", "LANG", "LC_ALL",
                          "CARGO_HOME", "RUSTUP_HOME", "GOPATH"] {
                if let Ok(val) = std::env::var(var) {
                    command.env(var, val);
                }
            }
        }
        match command.output() {
            Err(e) => (e.to_string(), true),
            Ok(out) => {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr),
                ).trim().to_string();
                let text = if text.is_empty() { "(no output)".into() } else { text };
                (text, !out.status.success())
            }
        }
    }
}

// ── I/O ──────────────────────────────────────────────────────────────────────

pub fn tools_dir(marlin_dir: &Path) -> PathBuf {
    let d = marlin_dir.join("tools");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load_all(marlin_dir: &Path) -> Vec<ExternalTool> {
    let dir = tools_dir(marlin_dir);
    let mut tools = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return tools };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") { continue; }
        if let Ok(data) = std::fs::read_to_string(&path) {
            match toml::from_str::<ExternalTool>(&data) {
                Ok(tool) => tools.push(tool),
                Err(e) => eprintln!("tools: parse error in {:?}: {e}", path.file_name()),
            }
        }
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
}

pub fn save_template(marlin_dir: &Path, name: &str) -> Result<PathBuf> {
    let tool = ExternalTool {
        name: name.to_string(),
        description: "Describe what this tool does".into(),
        properties: vec![ExternalToolProp {
            name: "input".into(),
            ty: "string".into(),
            description: "The main input text".into(),
            required: true,
        }],
        run: ExternalToolRun {
            kind: "shell".into(),
            command: "echo {input}".into(),
        },
    };
    let filename = format!("{}.toml", name.replace([' ', '/'], "_").to_lowercase());
    let path = tools_dir(marlin_dir).join(filename);
    std::fs::write(&path, toml::to_string_pretty(&tool)?)?;
    Ok(path)
}
