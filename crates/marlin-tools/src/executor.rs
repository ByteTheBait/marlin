use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::extract::extract_symbol;
use marlin_config::config::{AstMode, SandboxMode};

pub struct ToolResult {
    pub output: String,
    pub is_error: bool,
}

impl ToolResult {
    fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
        }
    }
    fn err(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: true,
        }
    }
}

// ~1 500 tokens of output before we spill to a log file
const LOG_THRESHOLD_BYTES: usize = 6_000;
// Hard cap on what we ever put into a tool result
const MAX_OUTPUT_BYTES: usize = 40_000;

/// Environment variables preserved across subprocess spawns when `clean_env` is set.
/// Single source of truth — previously duplicated (and drifted) across executor.rs,
/// external.rs, and the verify-command runner in engine/mod.rs.
pub const CLEAN_ENV_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LANG",
    "LC_ALL",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "GOPATH",
    "NODE_PATH",
    "npm_config_prefix",
];

/// Single-quote `s` for safe inclusion as one shell word, escaping embedded `'`.
/// Callers must place the placeholder bare (not already inside author-supplied
/// quotes) — this function supplies the only layer of quoting.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Callback used to search the code index for `search_codebase`.
pub type SearchFn<'a> = dyn Fn(&str, usize) -> String + 'a;
/// Callback used to search the code index for symbol definitions (`search_symbols`).
pub type SymbolSearchFn<'a> = dyn Fn(&str, usize) -> String + 'a;
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
    symbol_search_fn: Option<&SymbolSearchFn<'_>>,
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
                        let warning =
                            format!("// [AST/SEXPR] warning: {e} — degraded to raw text\n");
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
                        Path::new(&path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        extracted.len(),
                        content.len()
                    );
                    return ToolResult::ok(header + &extracted);
                }
                return ToolResult::ok(clamp(format!(
                    "// symbol {:?} not found — returning full file\n\n{}",
                    sym.trim(),
                    content
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

        // Multi-edit: apply several old→new replacements to one file in a single
        // call, in order. `edits` is an array of {old_string, new_string}, which
        // the string-only `parse_input` HashMap drops — so this arm re-parses the
        // raw JSON itself. Each old_string is matched against the current state
        // of the file (after prior edits in the same call), so edits can build on
        // one another like Claude/opencode's multi-edit.
        "multi_edit" => {
            let v: serde_json::Value = match serde_json::from_str(input_json) {
                Ok(v) => v,
                Err(e) => return ToolResult::err(format!("input parse error: {e}")),
            };
            let path = resolve(v["path"].as_str().unwrap_or(""));
            let edits =
                match v["edits"].as_array() {
                    Some(a) if !a.is_empty() => a,
                    Some(_) => return ToolResult::err("multi_edit 'edits' array is empty"),
                    None => return ToolResult::err(
                        "multi_edit requires an 'edits' array of {old_string, new_string} pairs",
                    ),
                };
            let original = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => return ToolResult::err(e.to_string()),
            };
            if let Some(snap) = snapshot_fn {
                snap(&path, "multi_edit");
            }
            let mut updated = original;
            let mut applied = 0usize;
            for (i, edit) in edits.iter().enumerate() {
                let old = edit["old_string"].as_str().unwrap_or("");
                let new = edit["new_string"].as_str().unwrap_or("");
                if old.is_empty() {
                    return ToolResult::err(format!(
                        "multi_edit edit {} has an empty old_string — use edit_file for inserts",
                        i + 1
                    ));
                }
                if !updated.contains(old) {
                    return ToolResult::err(format!(
                        "multi_edit: edit {} old_string not found in {path} ({applied} already applied)",
                        i + 1
                    ));
                }
                updated = updated.replacen(old, new, 1);
                applied += 1;
            }
            if let Err(e) = std::fs::write(&path, updated.as_bytes()) {
                return ToolResult::err(e.to_string());
            }
            ToolResult::ok(format!("multi_edit: applied {applied} edit(s) → {path}"))
        }

        "notebook_edit" => {
            let path = resolve(input.get("path").map(String::as_str).unwrap_or(""));
            let cell_id = input.get("cell_id").map(String::as_str).unwrap_or("");
            let cell_type = input.get("cell_type").map(String::as_str).unwrap_or("");
            let edit_mode_raw = input.get("edit_mode").map(String::as_str).unwrap_or("");
            let edit_mode = if edit_mode_raw.is_empty() {
                "replace"
            } else {
                edit_mode_raw
            };
            let new_source = input.get("new_source").map(String::as_str).unwrap_or("");

            let data = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => return ToolResult::err(e.to_string()),
            };
            let (msg, updated) =
                match build_notebook_edit(&data, cell_id, cell_type, edit_mode, new_source) {
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
                    Ok(out) => format_command_output(
                        &out.stdout,
                        &out.stderr,
                        out.status.success(),
                        logs_dir,
                    ),
                };
            }

            // Streaming path
            if let Some(stream) = stream_fn {
                let timeout_secs: u64 = input
                    .get("timeout")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(120)
                    .max(1);

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

                // Read stdout and stderr in background threads, feeding a shared
                // channel so the main loop can poll for the timeout while output
                // streams in. This replaces the old synchronous stderr read, which
                // blocked the whole call and made a timeout impossible.
                let (tx, rx) = std::sync::mpsc::channel::<(bool, String)>(); // (is_stderr, line)
                let tx_stderr = tx.clone();
                let stdout_thread = stdout.map(|out| {
                    std::thread::spawn(move || {
                        let reader = std::io::BufReader::new(out);
                        for line in reader.lines() {
                            if let Ok(l) = line {
                                let _ = tx.send((false, format!("{l}\n")));
                            }
                        }
                    })
                });
                let stderr_thread = stderr.map(|err| {
                    std::thread::spawn(move || {
                        let reader = std::io::BufReader::new(err);
                        for line in reader.lines() {
                            if let Ok(l) = line {
                                let _ = tx_stderr.send((true, format!("{l}\n")));
                            }
                        }
                    })
                });

                let mut stdout_buf = String::new();
                let mut stderr_buf = String::new();

                // Poll for completion, killing the child if it exceeds the timeout.
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
                loop {
                    // Drain whatever output has arrived so far.
                    while let Ok((is_err, chunk)) = rx.try_recv() {
                        if is_err {
                            stderr_buf.push_str(&chunk);
                        } else {
                            stream(&chunk);
                            stdout_buf.push_str(&chunk);
                        }
                    }

                    match child.try_wait() {
                        Ok(Some(status)) => {
                            // Child exited — drain any remaining output.
                            while let Ok((is_err, chunk)) = rx.recv() {
                                if is_err {
                                    stderr_buf.push_str(&chunk);
                                } else {
                                    stream(&chunk);
                                    stdout_buf.push_str(&chunk);
                                }
                            }
                            let combined = format!("{stdout_buf}{stderr_buf}");
                            let trimmed = combined.trim().to_string();
                            let result = if trimmed.is_empty() {
                                "(no output)".to_string()
                            } else {
                                trimmed
                            };
                            let success = status.success();
                            let display = format_command_output_display(&result, logs_dir, clamp);
                            return if success {
                                ToolResult::ok(display)
                            } else {
                                ToolResult::err(display)
                            };
                        }
                        Ok(None) => {
                            // Still running.
                            if std::time::Instant::now() >= deadline {
                                let _ = child.kill();
                                let _ = child.wait();
                                break;
                            }
                            std::thread::sleep(std::time::Duration::from_millis(20));
                        }
                        Err(e) => {
                            return ToolResult::err(format!("command wait error: {e}"));
                        }
                    }
                }

                // Timed out — drain whatever output arrived, then report.
                while let Ok((is_err, chunk)) = rx.try_recv() {
                    if is_err {
                        stderr_buf.push_str(&chunk);
                    } else {
                        stream(&chunk);
                        stdout_buf.push_str(&chunk);
                    }
                }
                if let Some(h) = stdout_thread {
                    let _ = h.join();
                }
                if let Some(h) = stderr_thread {
                    let _ = h.join();
                }

                let combined = format!("{stdout_buf}{stderr_buf}");
                let trimmed = combined.trim().to_string();
                let result = if trimmed.is_empty() {
                    "(no output)".to_string()
                } else {
                    trimmed
                };
                let display = format_command_output_display(&result, logs_dir, clamp);
                return ToolResult::err(format!(
                    "command timed out after {timeout_secs}s and was killed.\n{display}"
                ));
            }

            // Non-streaming fallback (used by subagents and tests)
            let timeout_secs: u64 = input
                .get("timeout")
                .and_then(|s| s.parse().ok())
                .unwrap_or(120)
                .max(1);

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

            // Read stdout and stderr in background threads so the main loop can
            // poll for the timeout even when the command produces no output
            // (e.g. `sleep 5`) — a synchronous read would block forever.
            use std::io::Read;
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            let (tx, rx) = std::sync::mpsc::channel::<(bool, Vec<u8>)>();
            let tx_err = tx.clone();
            let stdout_thread = stdout.map(|mut out| {
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let _ = out.read_to_end(&mut buf);
                    let _ = tx.send((false, buf));
                })
            });
            let stderr_thread = stderr.map(|mut err| {
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let _ = err.read_to_end(&mut buf);
                    let _ = tx_err.send((true, buf));
                })
            });

            let mut stdout_buf = Vec::new();
            let mut stderr_buf = Vec::new();

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
            loop {
                // Drain whatever output has arrived so far.
                while let Ok((is_err, chunk)) = rx.try_recv() {
                    if is_err {
                        stderr_buf.extend_from_slice(&chunk);
                    } else {
                        stdout_buf.extend_from_slice(&chunk);
                    }
                }

                match child.try_wait() {
                    Ok(Some(status)) => {
                        // Drain remaining output.
                        while let Ok((is_err, chunk)) = rx.recv() {
                            if is_err {
                                stderr_buf.extend_from_slice(&chunk);
                            } else {
                                stdout_buf.extend_from_slice(&chunk);
                            }
                        }
                        return format_command_output(
                            &stdout_buf,
                            &stderr_buf,
                            status.success(),
                            logs_dir,
                        );
                    }
                    Ok(None) => {
                        if std::time::Instant::now() >= deadline {
                            let _ = child.kill();
                            let _ = child.wait();
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    Err(e) => return ToolResult::err(format!("command wait error: {e}")),
                }
            }

            // Timed out — drain whatever output arrived.
            while let Ok((is_err, chunk)) = rx.try_recv() {
                if is_err {
                    stderr_buf.extend_from_slice(&chunk);
                } else {
                    stdout_buf.extend_from_slice(&chunk);
                }
            }
            if let Some(h) = stdout_thread {
                let _ = h.join();
            }
            if let Some(h) = stderr_thread {
                let _ = h.join();
            }
            let display = format_command_output(&stdout_buf, &stderr_buf, false, logs_dir);
            return ToolResult::err(format!(
                "command timed out after {timeout_secs}s and was killed.\n{}",
                display.output
            ));
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
            let limit: usize = input
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(5)
                .clamp(1, 20);
            ToolResult::ok(sf(query, limit))
        }

        "search_symbols" => {
            let Some(sf) = symbol_search_fn else {
                return ToolResult::err("index not built — run /index first");
            };
            let symbol = input.get("symbol").map(String::as_str).unwrap_or("").trim();
            if symbol.is_empty() {
                return ToolResult::err("search_symbols requires 'symbol'");
            }
            let limit: usize = input
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(5)
                .clamp(1, 20);
            ToolResult::ok(sf(symbol, limit))
        }

        "grep" => {
            let pattern = input
                .get("pattern")
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            if pattern.is_empty() {
                return ToolResult::err("grep requires 'pattern'");
            }
            let re = match regex::Regex::new(pattern) {
                Ok(r) => r,
                Err(e) => return ToolResult::err(format!("invalid regex: {e}")),
            };
            let path = input.get("path").map(String::as_str).unwrap_or("").trim();
            let root = if path.is_empty() {
                work_dir.to_string()
            } else {
                resolve(path)
            };
            let glob_pat = input.get("glob").map(String::as_str).unwrap_or("").trim();
            let context: usize = input
                .get("context")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let limit: usize = input
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(50)
                .clamp(1, 200);

            let root_path = Path::new(&root);
            if !root_path.exists() {
                return ToolResult::err(format!("path not found: {root}"));
            }

            let mut matches: Vec<String> = Vec::new();
            let mut files_searched = 0usize;
            let mut total_matches = 0usize;

            // Collect candidate files: a single file, or walk the directory tree.
            let mut files: Vec<PathBuf> = Vec::new();
            if root_path.is_file() {
                files.push(root_path.to_path_buf());
            } else {
                collect_grep_files(root_path, &mut files, &mut files_searched);
            }

            // Optional glob filter on the file path (relative to root).
            let glob_filter = if glob_pat.is_empty() {
                None
            } else {
                match glob::Pattern::new(glob_pat) {
                    Ok(p) => Some(p),
                    Err(e) => return ToolResult::err(format!("invalid glob: {e}")),
                }
            };

            for file in files {
                if total_matches >= limit {
                    break;
                }
                let rel = file
                    .strip_prefix(&root)
                    .unwrap_or(&file)
                    .to_string_lossy()
                    .to_string();
                if let Some(g) = &glob_filter {
                    if !g.matches(&rel) {
                        continue;
                    }
                }
                let Ok(text) = std::fs::read_to_string(&file) else {
                    continue;
                };
                let lines: Vec<&str> = text.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if total_matches >= limit {
                        break;
                    }
                    if re.is_match(line) {
                        total_matches += 1;
                        if context > 0 {
                            let start = i.saturating_sub(context);
                            let end = (i + context + 1).min(lines.len());
                            let mut block = String::new();
                            for j in start..end {
                                let marker = if j == i { ">" } else { " " };
                                block.push_str(&format!("  {marker} {:>4}: {}\n", j + 1, lines[j]));
                            }
                            matches.push(format!("{rel}:{i}:\n{block}"));
                        } else {
                            matches.push(format!("{rel}:{}: {}", i + 1, line));
                        }
                    }
                }
            }

            if matches.is_empty() {
                return ToolResult::ok(format!("No matches for /{pattern}/ in {root}"));
            }
            let mut out = format!(
                "grep /{pattern}/ — {} match(es) in {root}:\n",
                total_matches
            );
            for m in matches {
                out.push_str(&m);
                out.push('\n');
            }
            if total_matches >= limit {
                out.push_str(&format!("\n… truncated at {limit} matches"));
            }
            ToolResult::ok(clamp(out))
        }

        "glob" => {
            let pattern = input
                .get("pattern")
                .map(String::as_str)
                .unwrap_or("")
                .trim();
            if pattern.is_empty() {
                return ToolResult::err("glob requires 'pattern'");
            }
            let limit: usize = input
                .get("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(100)
                .clamp(1, 500);

            let pat = match glob::Pattern::new(pattern) {
                Ok(p) => p,
                Err(e) => return ToolResult::err(format!("invalid glob: {e}")),
            };

            let mut results: Vec<String> = Vec::new();
            collect_glob_matches(Path::new(work_dir), &pat, work_dir, &mut results, limit);

            if results.is_empty() {
                return ToolResult::ok(format!("No files match /{pattern}/"));
            }
            let mut out = format!("glob /{pattern}/ — {} result(s):\n", results.len());
            for r in &results {
                out.push_str(&format!("  {r}\n"));
            }
            if results.len() >= limit {
                out.push_str(&format!("\n… truncated at {limit} results"));
            }
            ToolResult::ok(clamp(out))
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
                        match compiler_run(&["compile", &file, "--lang", lang, "-o", &resolved_src])
                        {
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
        "macos" => "mxc-exec-mac",
        "windows" => "wxc-exec.exe",
        _ => "lxc-exec", // Linux and others
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
    let command_line = format!(
        "sh -c {}",
        serde_json::to_string(full_cmd.as_str()).unwrap()
    );

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
    let tmp = std::env::temp_dir().join(format!("marlin_mxc_{}.json", uuid::Uuid::new_v4()));
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
        Err(format!(
            "ast-compiler exit {}: {}",
            out.status,
            stderr.trim()
        ))
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
        Ok(if stdout.trim().is_empty() {
            "(no output)".into()
        } else {
            stdout
        })
    } else {
        Err(format!(
            "ast-harness exit {}: {}",
            out.status,
            stderr.trim()
        ))
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
    let mut nb: serde_json::Value =
        serde_json::from_str(notebook_json).map_err(|e| format!("invalid notebook JSON: {e}"))?;

    let msg = {
        let cells = nb
            .get_mut("cells")
            .and_then(|c| c.as_array_mut())
            .ok_or_else(|| "notebook has no 'cells' array".to_string())?;

        match edit_mode {
            "delete" => {
                let id = require_cell_id(cell_id, "delete")?;
                let idx =
                    find_cell_index(cells, id).ok_or_else(|| format!("no cell with id {id:?}"))?;
                cells.remove(idx);
                format!("deleted cell {id} ({} cells remain)", cells.len())
            }
            "insert" => {
                if cell_type != "code" && cell_type != "markdown" {
                    return Err(
                        "cell_type must be 'code' or 'markdown' for edit_mode=insert".into(),
                    );
                }
                let insert_at = if cell_id.is_empty() {
                    0
                } else {
                    find_cell_index(cells, cell_id)
                        .ok_or_else(|| format!("no cell with id {cell_id:?}"))?
                        + 1
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
                let idx =
                    find_cell_index(cells, id).ok_or_else(|| format!("no cell with id {id:?}"))?;
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
            other => {
                return Err(format!(
                    "unknown edit_mode {other:?} — valid: replace, insert, delete"
                ))
            }
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
    cells
        .iter()
        .position(|c| c.get("id").and_then(|v| v.as_str()) == Some(cell_id))
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
fn format_command_output(
    stdout: &[u8],
    stderr: &[u8],
    success: bool,
    logs_dir: Option<&Path>,
) -> ToolResult {
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    let trimmed = combined.trim().to_string();
    let result = if trimmed.is_empty() {
        "(no output)".to_string()
    } else {
        trimmed
    };

    let display = if result.len() > LOG_THRESHOLD_BYTES {
        match spill_to_log(&result, logs_dir) {
            Some(log_path) => {
                let total_lines = result.lines().count();
                let snippet: String = result
                    .lines()
                    .rev()
                    .take(40)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
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

/// Apply the log-spill / size-clamp display formatting to an already-combined
/// output string (used by the streaming run_command path, which assembles
/// stdout+stderr itself). Returns the display string.
fn format_command_output_display(
    result: &str,
    logs_dir: Option<&Path>,
    clamp: impl Fn(String) -> String,
) -> String {
    if result.len() > LOG_THRESHOLD_BYTES {
        match spill_to_log(result, logs_dir) {
            Some(log_path) => {
                let total_lines = result.lines().count();
                let snippet: String = result
                    .lines()
                    .rev()
                    .take(40)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "[Marlin: truncated {} lines of output. Full log saved to {}]\n\
                    --- last 40 lines ---\n{}",
                    total_lines, log_path, snippet
                )
            }
            None => clamp(result.to_string()),
        }
    } else {
        clamp(result.to_string())
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

pub fn resolve_path(p: &str, work_dir: &str) -> String {
    if p.is_empty() {
        return work_dir.to_string();
    }
    if p == "~" {
        return dirs::home_dir()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    }
    if Path::new(p).is_absolute() {
        return p.to_string();
    }
    Path::new(work_dir).join(p).to_string_lossy().to_string()
}

/// Directories never descended into by the grep/glob walkers — same spirit as
/// the index's SKIP_DIRS, kept local so these tools don't depend on the index.
const SEARCH_SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    ".next",
    "vendor",
    "__pycache__",
    ".venv",
    "venv",
    "build",
    ".cache",
    ".marlin",
];

/// Directories that are always skipped by the grep walker regardless of name
/// (e.g. hidden dirs like `.github` are fine to search, but `.git` is not).
fn is_skipped_dir(name: &str) -> bool {
    SEARCH_SKIP_DIRS.contains(&name)
}

/// Recursively collect searchable text files under `dir` into `files`.
/// `count` tracks how many entries were visited (for a rough "files searched"
/// figure). Skips binary files and junk directories.
fn collect_grep_files(dir: &Path, files: &mut Vec<PathBuf>, count: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !is_skipped_dir(&name) {
                collect_grep_files(&path, files, count);
            }
            continue;
        }
        *count += 1;
        if is_binary_path(&path) {
            continue;
        }
        files.push(path);
    }
}

/// Heuristic binary detection by extension — cheap and good enough for grep.
fn is_binary_path(path: &Path) -> bool {
    const BINARY_EXTS: &[&str] = &[
        "exe", "dll", "so", "dylib", "png", "jpg", "jpeg", "gif", "webp", "ico", "pdf", "zip",
        "tar", "gz", "wasm", "bin", "lock", "woff", "woff2", "ttf", "otf", "mp3", "mp4", "mov",
        "avi", "class", "pyc", "o", "a", "rlib",
    ];
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| BINARY_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Recursively walk `dir`, collecting paths (relative to `work_dir`) that match
/// `pat`. Stops once `results` reaches `limit`. Skips junk directories.
fn collect_glob_matches(
    dir: &Path,
    pat: &glob::Pattern,
    work_dir: &str,
    results: &mut Vec<String>,
    limit: usize,
) {
    if results.len() >= limit {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if results.len() >= limit {
            return;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if !is_skipped_dir(&name) {
                collect_glob_matches(&path, pat, work_dir, results, limit);
            }
            continue;
        }
        let rel = path
            .strip_prefix(work_dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if pat.matches(&rel) {
            results.push(rel);
        }
    }
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
        })
        .to_string()
    }

    fn cells(json: &str) -> Vec<serde_json::Value> {
        serde_json::from_str::<serde_json::Value>(json).unwrap()["cells"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn replace_updates_source_and_clears_stale_execution_state() {
        let (msg, out) =
            build_notebook_edit(&sample_notebook(), "b2", "", "replace", "print(3)\n").unwrap();
        assert!(msg.contains("replaced cell b2"));
        let cs = cells(&out);
        assert_eq!(cs[1]["source"], serde_json::json!(["print(3)\n"]));
        assert_eq!(cs[1]["execution_count"], serde_json::Value::Null);
        assert_eq!(cs[1]["outputs"], serde_json::json!([]));
    }

    #[test]
    fn replace_can_change_cell_type_and_drops_code_only_fields() {
        let (_, out) =
            build_notebook_edit(&sample_notebook(), "b2", "markdown", "replace", "notes").unwrap();
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
        let (msg, out) =
            build_notebook_edit(&sample_notebook(), "a1", "code", "insert", "print(0)").unwrap();
        assert!(msg.contains("inserted code cell"));
        let cs = cells(&out);
        assert_eq!(cs.len(), 3);
        assert_eq!(cs[1]["cell_type"], "code");
        assert_eq!(cs[1]["source"], serde_json::json!(["print(0)"]));
        assert_eq!(cs[2]["id"], "b2");
    }

    #[test]
    fn insert_without_cell_id_goes_to_the_start() {
        let (_, out) =
            build_notebook_edit(&sample_notebook(), "", "markdown", "insert", "intro").unwrap();
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
        })
        .to_string();
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

#[cfg(test)]
mod grep_glob_tests {
    use super::*;

    fn setup_dir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("marlin_grep_glob_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("src/main.rs"),
            "fn main() {\n    println!(\"hello world\");\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub fn helper() -> u32 { 42 }\n").unwrap();
        std::fs::write(dir.join("README.md"), "# Project\nhello there\n").unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        dir
    }

    fn allow_all() -> impl Fn(&str) -> bool {
        |_| true
    }

    #[test]
    fn grep_finds_matching_lines_with_line_numbers() {
        let dir = setup_dir();
        let input = serde_json::json!({"pattern": "hello"}).to_string();
        let res = execute(
            "grep",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(!res.is_error, "grep failed: {}", res.output);
        assert!(
            res.output.contains("main.rs:2"),
            "missing main.rs:2 in: {}",
            res.output
        );
        assert!(
            res.output.contains("README.md:2"),
            "missing README.md:2 in: {}",
            res.output
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn grep_respects_glob_filter() {
        let dir = setup_dir();
        let input = serde_json::json!({"pattern": "hello", "glob": "*.rs"}).to_string();
        let res = execute(
            "grep",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(!res.is_error);
        assert!(res.output.contains("main.rs:2"));
        assert!(
            !res.output.contains("README.md"),
            "glob filter should exclude README.md: {}",
            res.output
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn grep_no_match_returns_empty_message() {
        let dir = setup_dir();
        let input = serde_json::json!({"pattern": "zzz_nothing_here"}).to_string();
        let res = execute(
            "grep",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(!res.is_error);
        assert!(res.output.contains("No matches"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn grep_invalid_regex_errors() {
        let dir = setup_dir();
        let input = serde_json::json!({"pattern": "("}).to_string();
        let res = execute(
            "grep",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(res.is_error);
        assert!(res.output.contains("invalid regex"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn glob_finds_files_by_pattern() {
        let dir = setup_dir();
        let input = serde_json::json!({"pattern": "**/*.rs"}).to_string();
        let res = execute(
            "glob",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(!res.is_error, "glob failed: {}", res.output);
        assert!(
            res.output.contains("src/main.rs"),
            "missing src/main.rs in: {}",
            res.output
        );
        assert!(
            res.output.contains("src/lib.rs"),
            "missing src/lib.rs in: {}",
            res.output
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn glob_no_match_returns_empty_message() {
        let dir = setup_dir();
        let input = serde_json::json!({"pattern": "**/*.py"}).to_string();
        let res = execute(
            "glob",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(!res.is_error);
        assert!(res.output.contains("No files match"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn run_command_timeout_kills_and_reports() {
        let dir = setup_dir();
        // A command that sleeps longer than the 1s timeout.
        let input = serde_json::json!({"command": "sleep 5", "timeout": "1"}).to_string();
        let res = execute(
            "run_command",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(res.is_error, "expected timeout error, got: {}", res.output);
        assert!(
            res.output.contains("timed out"),
            "missing 'timed out' in: {}",
            res.output
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multi_edit_applies_all_edits_in_order() {
        let dir = setup_dir();
        let input = serde_json::json!({
            "path": "src/main.rs",
            "edits": [
                {"old_string": "fn main()", "new_string": "fn entry()"},
                {"old_string": "hello world", "new_string": "goodbye world"}
            ]
        })
        .to_string();
        let res = execute(
            "multi_edit",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(!res.is_error, "unexpected error: {}", res.output);
        assert!(
            res.output.contains("2 edit(s)"),
            "missing count in: {}",
            res.output
        );
        let content = std::fs::read_to_string(dir.join("src/main.rs")).unwrap();
        assert!(content.contains("fn entry()"), "content: {content}");
        assert!(content.contains("goodbye world"), "content: {content}");
        assert!(!content.contains("fn main()"), "content: {content}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multi_edit_error_when_old_string_missing() {
        let dir = setup_dir();
        let input = serde_json::json!({
            "path": "src/main.rs",
            "edits": [
                {"old_string": "does not exist", "new_string": "x"}
            ]
        })
        .to_string();
        let res = execute(
            "multi_edit",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(res.is_error, "expected error, got: {}", res.output);
        assert!(
            res.output.contains("not found"),
            "missing 'not found' in: {}",
            res.output
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn multi_edit_rejects_empty_old_string() {
        let dir = setup_dir();
        let input = serde_json::json!({
            "path": "src/main.rs",
            "edits": [
                {"old_string": "", "new_string": "x"}
            ]
        })
        .to_string();
        let res = execute(
            "multi_edit",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(res.is_error, "expected error, got: {}", res.output);
        assert!(
            res.output.contains("empty old_string"),
            "missing hint in: {}",
            res.output
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn run_command_completes_within_timeout() {
        let dir = setup_dir();
        let input = serde_json::json!({"command": "echo done", "timeout": "5"}).to_string();
        let res = execute(
            "run_command",
            &input,
            dir.to_str().unwrap(),
            &allow_all(),
            None,
            None,
            None,
            None,
            None,
            false,
            AstMode::Off,
            &SandboxMode::Off,
            &[],
        );
        assert!(!res.is_error, "expected success, got: {}", res.output);
        assert!(res.output.contains("done"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
