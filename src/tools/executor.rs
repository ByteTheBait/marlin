use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::extract::extract_symbol;
use crate::config::{AstMode, SandboxMode};

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

// ~1 500 tokens of output before we spill to a log file
const LOG_THRESHOLD_BYTES: usize = 6_000;
// Hard cap on what we ever put into a tool result
const MAX_OUTPUT_BYTES: usize = 40_000;

pub fn execute(
    name: &str,
    input_json: &str,
    work_dir: &str,
    is_allowed: &dyn Fn(&str) -> bool,
    search_fn: Option<&dyn Fn(&str, usize) -> String>,
    snapshot_fn: Option<&dyn Fn(&str, &str)>,
    logs_dir: Option<&Path>,
    clean_env: bool,
    ast_mode: AstMode,
    sandbox_mode: &SandboxMode,
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

            // SExpr mode: deliver compact AST S-expression instead of raw source
            if ast_mode == AstMode::SExpr {
                match run_ast_compiler_sexpr(&path) {
                    Ok(out) => return ToolResult::ok(clamp(out)),
                    Err(e) => {
                        // Graceful fallback: prepend warning, return raw text
                        let data = std::fs::read(&path)
                            .map(|d| String::from_utf8_lossy(&d).into_owned())
                            .unwrap_or_default();
                        let warning = format!(
                            "// [AST/SEXPR] warning: {e} — degraded to raw text\n"
                        );
                        return ToolResult::ok(clamp(format!("{warning}{data}")));
                    }
                }
            }

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
                    "not permitted: {cmd:?} — use /allow {first} or /sandbox [permissive|docker|gvisor]"
                ));
            }

            let output = if *sandbox_mode == SandboxMode::Mxc {
                run_in_mxc(cmd, work_dir)
            } else {
                let mut command = Command::new("sh");
                command.arg("-c").arg(cmd).current_dir(work_dir);
                if clean_env {
                    command.env_clear();
                    for var in &["PATH", "HOME", "USER", "LANG", "LC_ALL",
                                  "CARGO_HOME", "RUSTUP_HOME", "GOPATH",
                                  "NODE_PATH", "npm_config_prefix"] {
                        if let Ok(val) = std::env::var(var) {
                            command.env(var, val);
                        }
                    }
                }
                command.output()
            };

            match output {
                Err(e) => ToolResult::err(e.to_string()),
                Ok(out) => {
                    let combined = format!(
                        "{}{}",
                        String::from_utf8_lossy(&out.stdout),
                        String::from_utf8_lossy(&out.stderr)
                    );
                    let trimmed = combined.trim().to_string();
                    let result = if trimmed.is_empty() { "(no output)".to_string() } else { trimmed };

                    let (display, is_err) = if out.status.success() {
                        (result, false)
                    } else {
                        (result, true)
                    };

                    let display = if display.len() > LOG_THRESHOLD_BYTES {
                        match spill_to_log(&display, logs_dir) {
                            Some(log_path) => {
                                let total_lines = display.lines().count();
                                let snippet: String = display.lines()
                                    .rev().take(40).collect::<Vec<_>>()
                                    .into_iter().rev()
                                    .collect::<Vec<_>>().join("\n");
                                format!(
                                    "[Marlin: truncated {} lines of output. Full log saved to {}]\n\
                                    --- last 40 lines ---\n{}",
                                    total_lines, log_path, snippet
                                )
                            }
                            None => clamp(display),
                        }
                    } else {
                        clamp(display)
                    };

                    if is_err {
                        ToolResult::err(display)
                    } else {
                        ToolResult::ok(display)
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

        // ── AST Harness tools ────────────────────────────────────────────────

        "ast_skeleton" => {
            let file = resolve(input.get("file").map(String::as_str).unwrap_or(""));
            if file.is_empty() {
                return ToolResult::err("ast_skeleton requires 'file'");
            }
            match harness_run(&["skeleton", &file]) {
                Ok(out) => ToolResult::ok(clamp(out)),
                Err(e) => ToolResult::err(e),
            }
        }

        "ast_get_node" => {
            let file = resolve(input.get("file").map(String::as_str).unwrap_or(""));
            let node_id = input.get("node_id").map(String::as_str).unwrap_or("");
            if file.is_empty() || node_id.is_empty() {
                return ToolResult::err("ast_get_node requires 'file' and 'node_id'");
            }
            match harness_run(&["get", &file, node_id]) {
                Ok(out) => ToolResult::ok(clamp(out)),
                Err(e) => ToolResult::err(e),
            }
        }

        "ast_mutate" => {
            let file = resolve(input.get("file").map(String::as_str).unwrap_or(""));
            let node_id = input.get("node_id").map(String::as_str).unwrap_or("");
            let operation = input.get("operation").map(String::as_str).unwrap_or("");
            let lang = input.get("lang").map(String::as_str).unwrap_or("");
            let source_file = input.get("source_file").map(String::as_str).unwrap_or("");

            if file.is_empty() || node_id.is_empty() || operation.is_empty() {
                return ToolResult::err("ast_mutate requires 'file', 'node_id', and 'operation'");
            }

            // Dispatch to the correct ast-harness sub-command
            let mutate_result = match operation {
                "str-replace" => {
                    let old = input.get("old_json").map(String::as_str).unwrap_or("");
                    let new = input.get("new_json").map(String::as_str).unwrap_or("");
                    if old.is_empty() || new.is_empty() {
                        return ToolResult::err("str-replace requires 'old_json' and 'new_json'");
                    }
                    harness_run(&["str-replace", &file, node_id, old, new])
                }
                "append-stmt" => {
                    let stmt = input.get("statement_json").map(String::as_str).unwrap_or("");
                    if stmt.is_empty() {
                        return ToolResult::err("append-stmt requires 'statement_json'");
                    }
                    harness_run(&["append-stmt", &file, node_id, stmt])
                }
                "insert-before" => {
                    let index = input.get("index").map(String::as_str).unwrap_or("0");
                    let stmt = input.get("statement_json").map(String::as_str).unwrap_or("");
                    if stmt.is_empty() {
                        return ToolResult::err("insert-before requires 'statement_json'");
                    }
                    harness_run(&["insert-before", &file, node_id, index, stmt])
                }
                other => return ToolResult::err(format!(
                    "unknown ast_mutate operation {other:?} — valid: str-replace, append-stmt, insert-before"
                )),
            };

            match mutate_result {
                Err(e) => return ToolResult::err(format!("ast-harness failed: {e}")),
                Ok(mutate_out) => {
                    // Recompile source from the mutated AST JSON
                    let compile_out = if !lang.is_empty() && !source_file.is_empty() {
                        let resolved_src = resolve(source_file);
                        match compiler_run(&["compile", &file, "--lang", lang, "-o", &resolved_src]) {
                            Ok(out) => {
                                // Optimization pass (non-fatal if it fails)
                                let opt_note = match compiler_run(&["optimize", &file]) {
                                    Ok(_) => String::new(),
                                    Err(e) => format!("\n[optimize skipped: {e}]"),
                                };
                                format!("\n[compiled → {resolved_src}]{opt_note}\n{out}")
                            }
                            Err(e) => format!("\n[compile failed: {e}]"),
                        }
                    } else {
                        "\n[no lang/source_file — skipped recompile]".into()
                    };
                    ToolResult::ok(format!("{mutate_out}{compile_out}"))
                }
            }
        }

        _ => ToolResult::err(format!("unknown tool: {name}")),
    }
}

// ── MXC (Microsoft eXecution Containers) isolation ──────────────────────────

/// Platform-appropriate MXC native binary name.
pub fn mxc_binary_name() -> &'static str {
    mxc_binary()
}

fn mxc_binary() -> &'static str {
    match std::env::consts::OS {
        "macos"   => "mxc-exec-mac",
        "windows" => "wxc-exec.exe",
        _         => "lxc-exec",   // Linux and others
    }
}

/// Returns true if the MXC native binary is present in PATH.
pub fn detect_mxc() -> bool {
    // Command::new().output() returns Err(NotFound) when the binary doesn't exist.
    Command::new(mxc_binary()).arg("--help").output().is_ok()
}

/// Build an MXC JSON config that runs `cmd` inside a sandbox:
///   - workdir mounted read-write at its host path
///   - no outbound network
///   - 60-second timeout
fn mxc_config_json(cmd: &str, work_dir: &str) -> String {
    // cd into workdir first so relative paths in cmd resolve correctly.
    // Single-quote work_dir to handle spaces; escape any embedded single quotes.
    let safe_wd = work_dir.replace('\'', "'\\''");
    let full_cmd = format!("cd '{}' && {}", safe_wd, cmd);
    // serde_json::to_string produces a JSON-quoted string including surrounding '"'.
    let command_line = format!("sh -c {}", serde_json::to_string(full_cmd.as_str()).unwrap());

    serde_json::json!({
        "version": "0.7.0-alpha",
        "filesystem": {
            "readwritePaths": [work_dir]
        },
        "network": {
            "allowOutbound": false
        },
        "timeoutMs": 60000,
        "process": {
            "commandLine": command_line
        }
    })
    .to_string()
}

/// Execute `cmd` inside an MXC sandbox.
fn run_in_mxc(cmd: &str, work_dir: &str) -> std::io::Result<std::process::Output> {
    let json = mxc_config_json(cmd, work_dir);

    // Write the config to a temp file; MXC takes a file path argument.
    let tmp = std::env::temp_dir()
        .join(format!("marlin_mxc_{}.json", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, json.as_bytes())?;

    let result = Command::new(mxc_binary()).arg(&tmp).output();

    let _ = std::fs::remove_file(&tmp); // best-effort cleanup
    result
}

// ── AST subprocess helpers ───────────────────────────────────────────────────

/// Run `ast-compiler decompile <path> --format sexpr` and return stdout.
fn run_ast_compiler_sexpr(path: &str) -> Result<String, String> {
    compiler_run(&["decompile", path, "--format", "sexpr"])
}

/// Run `ast-compiler <args>` and return stdout, or an error string.
fn compiler_run(args: &[&str]) -> Result<String, String> {
    let out = Command::new("ast-compiler")
        .args(args)
        .output()
        .map_err(|e| format!("ast-compiler not found: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        if stdout.trim().is_empty() {
            Err("ast-compiler returned empty output".into())
        } else {
            Ok(stdout)
        }
    } else {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        Err(format!("ast-compiler exit {}: {}", out.status, stderr.trim()))
    }
}

/// Run `ast-harness <args>` and return stdout, or an error string.
fn harness_run(args: &[&str]) -> Result<String, String> {
    let out = Command::new("ast-harness")
        .args(args)
        .output()
        .map_err(|e| format!("ast-harness not found: {e}"))?;

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(if stdout.trim().is_empty() { "(no output)".into() } else { stdout })
    } else {
        Err(format!("ast-harness exit {}: {}", out.status, stderr.trim()))
    }
}

// ── Misc helpers ─────────────────────────────────────────────────────────────

fn spill_to_log(content: &str, logs_dir: Option<&Path>) -> Option<String> {
    let dir = logs_dir?;
    std::fs::create_dir_all(dir).ok()?;
    let id = uuid::Uuid::new_v4().to_string();
    let path = dir.join(format!("cmd_{id}.log"));
    std::fs::write(&path, content.as_bytes()).ok()?;
    Some(path.to_string_lossy().to_string())
}

fn parse_input(json: &str) -> Option<HashMap<String, String>> {
    if let Ok(m) = serde_json::from_str::<HashMap<String, String>>(json) {
        return Some(m);
    }
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
