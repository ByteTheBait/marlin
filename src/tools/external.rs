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

fn default_prop_type() -> String {
    "string".into()
}

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
            properties: self
                .properties
                .iter()
                .map(|p| ToolProp {
                    name: p.name.clone(),
                    ty: p.ty.clone(),
                    description: p.description.clone(),
                })
                .collect(),
            required: self
                .properties
                .iter()
                .filter(|p| p.required)
                .map(|p| p.name.clone())
                .collect(),
        }
    }

    /// Resolve the shell command by substituting declared property placeholders
    /// with shell-quoted values from the LLM's input. Only placeholders matching a
    /// *declared* property name are touched, so unrelated braces in the command
    /// (e.g. `jq '{name: .x}'`) survive untouched. A declared property the caller
    /// didn't supply resolves to an empty quoted string.
    pub fn resolved_command(&self, input: &HashMap<String, String>) -> String {
        let mut cmd = self.run.command.clone();
        for prop in &self.properties {
            let placeholder = format!("{{{}}}", prop.name);
            let value = input.get(&prop.name).cloned().unwrap_or_default();
            cmd = cmd.replace(&placeholder, &super::executor::shell_quote(&value));
        }
        cmd
    }

    /// Execute an already-resolved shell command (see `resolved_command`).
    /// Returns `(output, is_error)`.
    pub fn execute(
        &self,
        cmd: &str,
        work_dir: &str,
        clean_env: bool,
        sandbox_mode: &crate::config::SandboxMode,
    ) -> (String, bool) {
        use std::process::Command;

        let output = if *sandbox_mode == crate::config::SandboxMode::Mxc {
            super::executor::run_in_mxc(cmd, work_dir)
        } else {
            let mut command = Command::new("sh");
            command.arg("-c").arg(cmd).current_dir(work_dir);
            if clean_env {
                command.env_clear();
                for var in super::executor::CLEAN_ENV_VARS {
                    if let Ok(val) = std::env::var(var) {
                        command.env(var, val);
                    }
                }
            }
            command.output()
        };

        match output {
            Err(e) => (e.to_string(), true),
            Ok(out) => {
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr),
                )
                .trim()
                .to_string();
                let text = if text.is_empty() {
                    "(no output)".into()
                } else {
                    text
                };
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
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return tools;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
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

// ── Default tools ────────────────────────────────────────────────────────────

/// Install one working example tool on first run — same write-if-missing
/// pattern as `skills::install_defaults`. Never overwrites a user's file.
pub fn install_defaults(marlin_dir: &Path) {
    let dir = tools_dir(marlin_dir);
    for tool in default_tools() {
        let path = dir.join(format!("{}.toml", tool.name));
        if !path.exists() {
            if let Ok(data) = toml::to_string_pretty(&tool) {
                let _ = std::fs::write(path, data);
            }
        }
    }
}

fn default_tools() -> Vec<ExternalTool> {
    vec![ExternalTool {
        name: "git_log".into(),
        description: "Show recent commits touching a given file".into(),
        properties: vec![ExternalToolProp {
            name: "path".into(),
            ty: "string".into(),
            description: "File path relative to the repo root".into(),
            required: true,
        }],
        run: ExternalToolRun {
            kind: "shell".into(),
            command: "git log --oneline -n 10 -- {path}".into(),
        },
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jq_braces_survive_substitution_unmangled() {
        // Regression test: the old brace-stripping substitution deleted
        // everything between ANY `{` and `}`, silently gutting jq/awk programs
        // that aren't a declared placeholder.
        let tool = ExternalTool {
            name: "jq_run".into(),
            description: "run jq".into(),
            properties: vec![ExternalToolProp {
                name: "path".into(),
                ty: "string".into(),
                description: String::new(),
                required: true,
            }],
            run: ExternalToolRun {
                kind: "shell".into(),
                command: "jq '{name: .x}' {path}".into(),
            },
        };
        let mut input = HashMap::new();
        input.insert("path".to_string(), "data.json".to_string());
        let cmd = tool.resolved_command(&input);
        assert_eq!(cmd, "jq '{name: .x}' 'data.json'");
    }

    #[test]
    fn declared_placeholder_substitutes_as_quoted_word() {
        let tool = ExternalTool {
            name: "greet".into(),
            description: String::new(),
            properties: vec![ExternalToolProp {
                name: "name".into(),
                ty: "string".into(),
                description: String::new(),
                required: true,
            }],
            run: ExternalToolRun {
                kind: "shell".into(),
                command: "echo {name}".into(),
            },
        };
        let mut input = HashMap::new();
        input.insert("name".to_string(), "a; rm -rf ~".to_string());
        let cmd = tool.resolved_command(&input);
        assert_eq!(cmd, "echo 'a; rm -rf ~'");
    }
}
