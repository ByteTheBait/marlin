use std::collections::HashMap;
use std::path::Path;
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

/// Environment variables preserved across subprocess spawns when `clean_env` is set.
/// Single source of truth — previously duplicated (and drifted) across executor.rs,
/// external.rs, and the verify-command runner in engine/mod.rs.
pub(crate) const CLEAN_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "USER", "LANG", "LC_ALL",
    "CARGO_HOME", "RUSTUP_HOME", "GOPATH",
    "NODE_PATH", "npm_config_prefix",
];

/// Single-quote `s` for safe inclusion as one shell word, escaping embedded `'`.
/// Callers must place the placeholder bare (not already inside author-supplied
/// quotes) — this function supplies the only layer of quoting.
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Callback used to search the code index for `search_codebase`.
pub type SearchFn<'a> = dyn Fn(&str, usize) -> String + 'a;
/// Callback used to snapshot a file before a write/edit tool mutates it.
pub type SnapshotFn<'a> = dyn Fn(&str, &str) + 'a;
/// Callback used to stream run_command output chunks as they arrive.
pub type StreamFn<'a> = dyn Fn(&str) + 'a;

#[allow(clippy::too_many_arguments)]
pub fn execute(
    name: &str,
    input_json: &str,
    work_dir: &str,
    is_allowed: &dyn Fn(&str) -> bool,
    search_fn: Option<&SearchFn<'_>>,
    snapshot_fn: Option<&SnapshotFn<'_>>,
    stream_fn: Option<&StreamFn<'_>>,
    logs_dir: Option<&Path>,
    clean_env: bool,
    ast_mode: AstMode,
    sandbox_mode: &SandboxMode,
    external_tools: &[super::external::ExternalTool],
) -> ToolResult {
    let input: HashMap<String, String> = match parse_input(input_json) {
        Some(m) => m,
        None => return ToolResult::err(format!("input parse error: {input_json}")),
    };

    let resolve = |p: &str| resolve_path(p, work_dir);
    let clamp = |s: String| -> String {
        if s.len() > MAX_OUTPUT_BYTES {
            // Back off to the nearest char boundary — slicing mid-UTF-8-char panics.
            let mut end = MAX_OUTPUT_BYTES;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}\n…(truncated)", &s[..end])
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

        "notebook_edit" => {
            let path = resolve(input.get("path").map(String::as_str).unwrap_or(""));
            let cell_id = input.get("cell_id").map(String::as_str).unwrap_or("");
            let cell_type = input.get("cell_type").map(String::as_str).unwrap_or("");
            let edit_mode_raw = input.get("edit_mode").map(String::as_str).unwrap_or("");
            let edit_mode = if edit_mode_raw.is_empty() { "replace" } else { edit_mode_raw };
            let new_source = input.get("new_source").map(String::as_str).unwrap_or("");

            let data = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => return ToolResult::err(e.to_string()),
            };
            let (msg, updated) = match build_notebook_edit(&data, cell_id, cell_type, edit_mode, new_source) {
                Ok(v) => v,
                Err(e) => return ToolResult::err(e),
            };
            if let Some(snap) = snapshot_fn {
                snap(&path, "notebook_edit");
            }
            if let Err(e) = std::fs::write(&path, updated.as_bytes()) {
                return ToolResult::err(e.to_string());
            }
            ToolResult::ok(format!("{msg} → {path}"))
        }

        "run_command" => {
            let cmd = input.get("command").map(String::as_str).unwrap_or("");
            if !is_allowed(cmd) {
                let first = cmd.split_whitespace().next().unwrap_or(cmd);
                return ToolResult::err(format!(
                    "not permitted: {cmd:?} — use /allow {first} or /sandbox [permissive|docker|gvisor]"
                ));
            }

            // MXC path — no streaming support
            if *sandbox_mode == SandboxMode::Mxc {
                return match run_in_mxc(cmd, work_dir) {
                    Err(e) => ToolResult::err(e.to_string()),
                    Ok(out) => format_command_output(&out.stdout, &out.stderr, out.status.success(), logs_dir),
                };
            }

            // Streaming path
            if let Some(stream) = stream_fn {
                let mut command = Command::new("sh");
                command.arg("-c").arg(cmd).current_dir(work_dir);
                if clean_env {
                    command.env_clear();
                    for var in CLEAN_ENV_VARS {
                        if let Ok(val) = std::env::var(var) {
                            command.env(var, val);
                        }
                    }
                }
                command.stdout(std::process::Stdio::piped());
                command.stderr(std::process::Stdio::piped());

                let mut child = match command.spawn() {
                    Ok(c) => c,
                    Err(e) => return ToolResult::err(e.to_string()),
                };

                use std::io::BufRead;
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();

                // Read stdout in a thread, send chunks through the callback
                let (tx, rx) = std::sync::mpsc::channel::<String>();
                let stdout_thread = stdout.map(|out| {
                    std::thread::spawn(move || {
                        let reader = std::io::BufReader::new(out);
                        for line in reader.lines() {
                            if let Ok(l) = line {
                                let _ = tx.send(format!("{l}\n"));
                            }
                        }
                    })
                });

                let mut stderr_buf = String::new();
                if let Some(err) = stderr {
                    let reader = std::io::BufReader::new(err);
                    for line in reader.lines() {
                        if let Ok(l) = line {
                            stderr_buf.push_str(&l);
                            stderr_buf.push('\n');
                        }
                    }
                }

                let mut stdout_buf = String::new();
                for chunk in rx {
                    stream(&chunk);
                    stdout_buf.push_str(&chunk);
                }
                if let Some(h) = stdout_thread {
                    let _ = h.join();
                }

                let status = child.wait();
                let combined = format!("{stdout_buf}{stderr_buf}");
                let trimmed = combined.trim().to_string();
                let result = if trimmed.is_empty() { "(no output)".to_string() } else { trimmed };
                let success = status.map(|s| s.success()).unwrap_or(false);

                let display = if result.len() > LOG_THRESHOLD_BYTES {
                    match spill_to_log(&result, logs_dir) {
                        Some(log_path) => {
                            let total_lines = result.lines().count();
                            let snippet: String = result.lines()
                                .rev().take(40).collect::<Vec<_>>()
                                .into_iter().rev()
                                .collect::<Vec<_>>().join("\n");
                            format!(
                                "[Marlin: truncated {} lines of output. Full log saved to {}]\n\
                                --- last 40 lines ---\n{}",
                                total_lines, log_path, snippet
                            )
                        }
                        None => clamp(result),
                    }
                } else {
                    clamp(result)
                };

                return if success {
                    ToolResult::ok(display)
                } else {
                    ToolResult::err(display)
                };
            }

            // Non-streaming fallback
            let mut command = Command::new("sh");
            command.arg("-c").arg(cmd).current_dir(work_dir);
            if clean_env {
                command.env_clear();
                for var in CLEAN_ENV_VARS {
                    if let Ok(val) = std::env::var(var) {
                        command.env(var, val);
                    }
                }
            }
            match command.output() {
                Err(e) => ToolResult::err(e.to_string()),
                Ok(out) => format_command_output(&out.stdout, &out.stderr, out.status.success(), logs_dir),
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
                .clamp(1, 20);
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
                Err(e) => ToolResult::err(format!("ast-harness failed: {e}")),
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

        _ => {
            // Try user-defined external tools from ~/.marlin/tools/*.toml.
            if let Some(et) = external_tools.iter().find(|t| t.name == name) {
                let cmd = et.resolved_command(&input);
                if !is_allowed(&cmd) {
                    let first = cmd.split_whitespace().next().unwrap_or(&cmd);
                    return ToolResult::err(format!(
                        "not permitted: {cmd:?} — use /allow {first} or /sandbox [permissive|docker|gvisor]"
                    ));
                }
                let (output, is_error) = et.execute(&cmd, work_dir, clean_env, sandbox_mode);
                return if is_error {
                    ToolResult::err(output)
                } else {
                    ToolResult::ok(output)
                };
            }
            ToolResult::err(format!("unknown tool: {name}"))
        }
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
    let full_cmd = format!("cd {} && {}", shell_quote(work_dir), cmd);
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
pub(crate) fn run_in_mxc(cmd: &str, work_dir: &str) -> std::io::Result<std::process::Output> {
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

// ── Notebook (.ipynb) helpers ────────────────────────────────────────────────

/// Apply a replace/insert/delete edit to a parsed notebook's `cells` array and
/// return `(summary message, re-serialized notebook JSON)` — never writes to
/// disk itself, so the caller can snapshot the file right before overwriting it,
/// same as `edit_file`.
fn build_notebook_edit(
    notebook_json: &str,
    cell_id: &str,
    cell_type: &str,
    edit_mode: &str,
    new_source: &str,
) -> Result<(String, String), String> {
    let mut nb: serde_json::Value = serde_json::from_str(notebook_json)
        .map_err(|e| format!("invalid notebook JSON: {e}"))?;

    let msg = {
        let cells = nb.get_mut("cells")
            .and_then(|c| c.as_array_mut())
            .ok_or_else(|| "notebook has no 'cells' array".to_string())?;

        match edit_mode {
            "delete" => {
                let id = require_cell_id(cell_id, "delete")?;
                let idx = find_cell_index(cells, id)
                    .ok_or_else(|| format!("no cell with id {id:?}"))?;
                cells.remove(idx);
                format!("deleted cell {id} ({} cells remain)", cells.len())
            }
            "insert" => {
                if cell_type != "code" && cell_type != "markdown" {
                    return Err("cell_type must be 'code' or 'markdown' for edit_mode=insert".into());
                }
                let insert_at = if cell_id.is_empty() {
                    0
                } else {
                    find_cell_index(cells, cell_id)
                        .ok_or_else(|| format!("no cell with id {cell_id:?}"))? + 1
                };
                let new_id = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
                let mut new_cell = serde_json::json!({
                    "cell_type": cell_type,
                    "metadata": {},
                    "source": source_lines(new_source),
                    "id": new_id,
                });
                if cell_type == "code" {
                    new_cell["execution_count"] = serde_json::Value::Null;
                    new_cell["outputs"] = serde_json::Value::Array(vec![]);
                }
                cells.insert(insert_at, new_cell);
                format!("inserted {cell_type} cell {new_id} at position {insert_at}")
            }
            "replace" => {
                let id = require_cell_id(cell_id, "replace")?;
                let idx = find_cell_index(cells, id)
                    .ok_or_else(|| format!("no cell with id {id:?}"))?;
                let cell = &mut cells[idx];
                cell["source"] = serde_json::Value::Array(source_lines(new_source));
                let target_type = if cell_type.is_empty() {
                    cell["cell_type"].as_str().unwrap_or("code").to_string()
                } else {
                    cell_type.to_string()
                };
                if cell["cell_type"].as_str() != Some(target_type.as_str()) {
                    cell["cell_type"] = serde_json::Value::String(target_type.clone());
                }
                if target_type == "code" {
                    cell["execution_count"] = serde_json::Value::Null;
                    cell["outputs"] = serde_json::Value::Array(vec![]);
                } else if let Some(obj) = cell.as_object_mut() {
                    obj.remove("execution_count");
                    obj.remove("outputs");
                }
                format!("replaced cell {id}")
            }
            other => return Err(format!(
                "unknown edit_mode {other:?} — valid: replace, insert, delete"
            )),
        }
    };

    let out = serde_json::to_string_pretty(&nb).map_err(|e| e.to_string())?;
    Ok((msg, out))
}

fn require_cell_id<'a>(cell_id: &'a str, mode: &str) -> Result<&'a str, String> {
    if cell_id.is_empty() {
        Err(format!("cell_id is required for edit_mode={mode}"))
    } else {
        Ok(cell_id)
    }
}

/// Match by the cell's `id` field first; falls back to treating `cell_id` as a
/// 0-based index for notebooks predating nbformat 4.5 cell ids.
fn find_cell_index(cells: &[serde_json::Value], cell_id: &str) -> Option<usize> {
    cells.iter().position(|c| c.get("id").and_then(|v| v.as_str()) == Some(cell_id))
        .or_else(|| cell_id.parse::<usize>().ok().filter(|&i| i < cells.len()))
}

/// Split into nbformat's line-array source representation: every line but the
/// last keeps its trailing `\n`, matching how Jupyter itself stores `source`.
fn source_lines(s: &str) -> Vec<serde_json::Value> {
    if s.is_empty() {
        return vec![];
    }
    let mut out = Vec::new();
    let mut start = 0;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            out.push(serde_json::Value::String(s[start..=i].to_string()));
            start = i + 1;
        }
    }
    if start < s.len() {
        out.push(serde_json::Value::String(s[start..].to_string()));
    }
    out
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

/// Format command output bytes into a ToolResult, applying size limits.
fn format_command_output(stdout: &[u8], stderr: &[u8], success: bool, logs_dir: Option<&Path>) -> ToolResult {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    let trimmed = combined.trim().to_string();
    let result = if trimmed.is_empty() { "(no output)".to_string() } else { trimmed };

    let display = if result.len() > LOG_THRESHOLD_BYTES {
        match spill_to_log(&result, logs_dir) {
            Some(log_path) => {
                let total_lines = result.lines().count();
                let snippet: String = result.lines()
                    .rev().take(40).collect::<Vec<_>>()
                    .into_iter().rev()
                    .collect::<Vec<_>>().join("\n");
                format!(
                    "[Marlin: truncated {} lines of output. Full log saved to {}]\n\
                    --- last 40 lines ---\n{}",
                    total_lines, log_path, snippet
                )
            }
            None => clamp(result),
        }
    } else {
        clamp(result)
    };

    if success {
        ToolResult::ok(display)
    } else {
        ToolResult::err(display)
    }
}

fn clamp(s: String) -> String {
    if s.len() > MAX_OUTPUT_BYTES {
        let mut end = MAX_OUTPUT_BYTES;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}\n…(truncated)", &s[..end])
    } else {
        s
    }
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
                // Numbers are common for things like {"limit": 10} — convert
                // them so they parse correctly downstream.
                serde_json::Value::Number(n) => n.to_string(),
                // Booleans and null don't have a natural string representation
                // in tool inputs — skip them rather than silently producing
                // "true"/"false"/"null" which tools won't expect.
                _ => continue,
            };
            out.insert(k, s);
        }
        return Some(out);
    }
    None
}

pub(crate) fn resolve_path(p: &str, work_dir: &str) -> String {
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

#[cfg(test)]
mod notebook_edit_tests {
    use super::*;

    fn sample_notebook() -> String {
        serde_json::json!({
            "cells": [
                {"cell_type": "markdown", "metadata": {}, "id": "a1", "source": ["# Title\n"]},
                {"cell_type": "code", "metadata": {}, "id": "b2", "execution_count": 3,
                 "outputs": ["stale"], "source": ["print(1)\n", "print(2)"]},
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5,
        }).to_string()
    }

    fn cells(json: &str) -> Vec<serde_json::Value> {
        serde_json::from_str::<serde_json::Value>(json).unwrap()["cells"].as_array().unwrap().clone()
    }

    #[test]
    fn replace_updates_source_and_clears_stale_execution_state() {
        let (msg, out) = build_notebook_edit(&sample_notebook(), "b2", "", "replace", "print(3)\n").unwrap();
        assert!(msg.contains("replaced cell b2"));
        let cs = cells(&out);
        assert_eq!(cs[1]["source"], serde_json::json!(["print(3)\n"]));
        assert_eq!(cs[1]["execution_count"], serde_json::Value::Null);
        assert_eq!(cs[1]["outputs"], serde_json::json!([]));
    }

    #[test]
    fn replace_can_change_cell_type_and_drops_code_only_fields() {
        let (_, out) = build_notebook_edit(&sample_notebook(), "b2", "markdown", "replace", "notes").unwrap();
        let cs = cells(&out);
        assert_eq!(cs[1]["cell_type"], "markdown");
        assert!(cs[1].get("execution_count").is_none());
        assert!(cs[1].get("outputs").is_none());
    }

    #[test]
    fn replace_without_cell_id_errors() {
        assert!(build_notebook_edit(&sample_notebook(), "", "", "replace", "x").is_err());
    }

    #[test]
    fn replace_unknown_cell_id_errors() {
        assert!(build_notebook_edit(&sample_notebook(), "nope", "", "replace", "x").is_err());
    }

    #[test]
    fn insert_requires_a_valid_cell_type() {
        assert!(build_notebook_edit(&sample_notebook(), "a1", "", "insert", "x").is_err());
        assert!(build_notebook_edit(&sample_notebook(), "a1", "raw", "insert", "x").is_err());
    }

    #[test]
    fn insert_after_given_cell_id() {
        let (msg, out) = build_notebook_edit(&sample_notebook(), "a1", "code", "insert", "print(0)").unwrap();
        assert!(msg.contains("inserted code cell"));
        let cs = cells(&out);
        assert_eq!(cs.len(), 3);
        assert_eq!(cs[1]["cell_type"], "code");
        assert_eq!(cs[1]["source"], serde_json::json!(["print(0)"]));
        assert_eq!(cs[2]["id"], "b2");
    }

    #[test]
    fn insert_without_cell_id_goes_to_the_start() {
        let (_, out) = build_notebook_edit(&sample_notebook(), "", "markdown", "insert", "intro").unwrap();
        let cs = cells(&out);
        assert_eq!(cs.len(), 3);
        assert_eq!(cs[0]["cell_type"], "markdown");
        assert_eq!(cs[0]["source"], serde_json::json!(["intro"]));
        assert_eq!(cs[1]["id"], "a1");
    }

    #[test]
    fn delete_removes_the_matching_cell() {
        let (msg, out) = build_notebook_edit(&sample_notebook(), "a1", "", "delete", "").unwrap();
        assert!(msg.contains("deleted cell a1"));
        let cs = cells(&out);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0]["id"], "b2");
    }

    #[test]
    fn delete_without_cell_id_errors() {
        assert!(build_notebook_edit(&sample_notebook(), "", "", "delete", "").is_err());
    }

    #[test]
    fn unknown_edit_mode_errors() {
        assert!(build_notebook_edit(&sample_notebook(), "b2", "", "bogus", "x").is_err());
    }

    #[test]
    fn malformed_json_errors() {
        assert!(build_notebook_edit("not json", "b2", "", "replace", "x").is_err());
    }

    #[test]
    fn falls_back_to_index_when_cells_have_no_id() {
        let nb = serde_json::json!({
            "cells": [
                {"cell_type": "code", "metadata": {}, "source": ["a = 1"]},
                {"cell_type": "code", "metadata": {}, "source": ["b = 2"]},
            ],
        }).to_string();
        let (_, out) = build_notebook_edit(&nb, "1", "", "replace", "b = 3").unwrap();
        let cs = cells(&out);
        assert_eq!(cs[1]["source"], serde_json::json!(["b = 3"]));
        assert_eq!(cs[0]["source"], serde_json::json!(["a = 1"]));
    }

    #[test]
    fn source_lines_preserves_trailing_newline_semantics() {
        assert_eq!(source_lines(""), Vec::<serde_json::Value>::new());
        assert_eq!(
            source_lines("a\nb"),
            vec![serde_json::json!("a\n"), serde_json::json!("b")]
        );
        assert_eq!(
            source_lines("a\nb\n"),
            vec![serde_json::json!("a\n"), serde_json::json!("b\n")]
        );
    }
}
