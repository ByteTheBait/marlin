use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::extract::extract_symbol;

pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

impl ToolResult {
    fn ok(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: false }
    }
    fn err(output: impl Into<String>) -> Self {
        Self { output: output.into(), is_error: true }
    }
}

const MAX_OUTPUT_BYTES: usize = 40_000;

pub fn execute(
    name: &str,
    input_json: &str,
    work_dir: &str,
    is_allowed: &dyn Fn(&str) -> bool,
    search_fn: Option<&dyn Fn(&str, usize) -> String>,
    snapshot_fn: Option<&dyn Fn(&str, &str)>,
) -> ToolResult {
    let input: HashMap<String, String> = match parse_input(input_json) {
        Some(m) => m,
        None => return ToolResult::err(format!("input parse error: {input_json}")),
    };

    let resolve = |p: &str| resolve_path(p, work_dir);
    let clamp = |s: String| -> String {
        if s.len() > MAX_OUTPUT_BYTES {
            format!("{}\n…(truncated)", &s[..MAX_OUTPUT_BYTES])
        } else {
            s
        }
    };

    match name {
        "read_file" => {
            let path = resolve(input.get("path").map(String::as_str).unwrap_or(""));
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(e) => return ToolResult::err(e.to_string()),
            };
            let content = String::from_utf8_lossy(&data).into_owned();

            if let Some(sym) = input.get("function").filter(|s| !s.trim().is_empty()) {
                let ext = Path::new(&path)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if let Some(extracted) = extract_symbol(&content, sym.trim(), &ext) {
                    let header = format!(
                        "// extracted: {} from {} ({} of {} bytes)\n",
                        sym.trim(),
                        Path::new(&path).file_name().unwrap_or_default().to_string_lossy(),
                        extracted.len(),
                        content.len()
                    );
                    return ToolResult::ok(header + &extracted);
                }
                // Symbol not found — return full file with notice
                return ToolResult::ok(clamp(format!(
                    "// symbol {:?} not found — returning full file\n\n{}",
                    sym.trim(), content
                )));
            }
            ToolResult::ok(clamp(content))
        }

        "write_file" => {
            let path = resolve(input.get("path").map(String::as_str).unwrap_or(""));
            let content = input.get("content").map(String::as_str).unwrap_or("");
            if let Some(snap) = snapshot_fn {
                snap(&path, "write_file");
            }
            if let Some(parent) = Path::new(&path).parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    return ToolResult::err(e.to_string());
                }
            }
            if let Err(e) = std::fs::write(&path, content.as_bytes()) {
                return ToolResult::err(e.to_string());
            }
            ToolResult::ok(format!("wrote {} bytes → {}", content.len(), path))
        }

        "edit_file" => {
            let path = resolve(input.get("path").map(String::as_str).unwrap_or(""));
            let old = input.get("old_string").map(String::as_str).unwrap_or("");
            let new = input.get("new_string").map(String::as_str).unwrap_or("");
            let original = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => return ToolResult::err(e.to_string()),
            };
            if !original.contains(old) {
                return ToolResult::err("old_string not found in file");
            }
            if let Some(snap) = snapshot_fn {
                snap(&path, "edit_file");
            }
            let updated = original.replacen(old, new, 1);
            if let Err(e) = std::fs::write(&path, updated.as_bytes()) {
                return ToolResult::err(e.to_string());
            }
            ToolResult::ok(format!("edited {path}"))
        }

        "run_command" => {
            let cmd = input.get("command").map(String::as_str).unwrap_or("");
            if !is_allowed(cmd) {
                let first = cmd.split_whitespace().next().unwrap_or(cmd);
                return ToolResult::err(format!(
                    "not permitted: {cmd:?} — use /allow {first} or /sandbox on for autonomous mode"
                ));
            }
            let output = Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(work_dir)
                .output();
            match output {
                Err(e) => ToolResult::err(e.to_string()),
                Ok(out) => {
                    let combined = format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let trimmed = clamp(combined.trim().to_string());
                    let result = if trimmed.is_empty() { "(no output)".to_string() } else { trimmed };
                    if out.status.success() {
                        ToolResult::ok(result)
                    } else {
                        ToolResult::err(result)
                    }
                }
            }
        }

        "list_directory" => {
            let dir = if let Some(p) = input.get("path").filter(|p| !p.is_empty()) {
                resolve(p)
            } else {
                work_dir.to_string()
            };
            match std::fs::read_dir(&dir) {
                Err(e) => ToolResult::err(e.to_string()),
                Ok(entries) => {
                    let mut lines: Vec<String> = entries
                        .filter_map(|e| e.ok())
                        .map(|e| {
                            let name = e.file_name().to_string_lossy().to_string();
                            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                name + "/"
                            } else {
                                name
                            }
                        })
                        .collect();
                    lines.sort();
                    if lines.is_empty() {
                        ToolResult::ok("(empty directory)")
                    } else {
                        ToolResult::ok(lines.join("\n"))
                    }
                }
            }
        }

        "create_directory" => {
            let path = resolve(input.get("path").map(String::as_str).unwrap_or(""));
            match std::fs::create_dir_all(&path) {
                Ok(_) => ToolResult::ok(format!("created {path}")),
                Err(e) => ToolResult::err(e.to_string()),
            }
        }

        "search_codebase" => {
            let Some(sf) = search_fn else {
                return ToolResult::err("index not built — run /index first");
            };
            let query = input.get("query").map(String::as_str).unwrap_or("").trim();
            if query.is_empty() {
                return ToolResult::err("query is required");
            }
            let limit: usize = input.get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(5)
                .max(1)
                .min(20);
            ToolResult::ok(sf(query, limit))
        }

        _ => ToolResult::err(format!("unknown tool: {name}")),
    }
}

fn parse_input(json: &str) -> Option<HashMap<String, String>> {
    if let Ok(m) = serde_json::from_str::<HashMap<String, String>>(json) {
        return Some(m);
    }
    // Fallback: coerce values to strings
    if let Ok(raw) = serde_json::from_str::<HashMap<String, serde_json::Value>>(json) {
        let mut out = HashMap::new();
        for (k, v) in raw {
            let s = match v {
                serde_json::Value::String(s) => s,
                other => other.to_string(),
            };
            out.insert(k, s);
        }
        return Some(out);
    }
    None
}

fn resolve_path(p: &str, work_dir: &str) -> String {
    if p.is_empty() { return work_dir.to_string(); }
    if p == "~" {
        return dirs::home_dir().unwrap_or_default().to_string_lossy().to_string();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    if Path::new(p).is_absolute() { return p.to_string(); }
    Path::new(work_dir).join(p).to_string_lossy().to_string()
}
