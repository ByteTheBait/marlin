//! Single mandatory funnel that every tool invocation passes through before it
//! touches the filesystem or a shell — regardless of whether the call came
//! from a built-in tool, a user skill, an external tool, or a user-defined
//! slash command. Closes the bypass where skills and external tools ran with
//! an unconditional allow-all predicate and a hardcoded `SandboxMode::Off`.

use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::skills::Skill;
use crate::tools::{executor, policy};

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// Nothing to flag — proceed.
    Allow,
    /// Not an unconditional block, but risky enough to require interactive
    /// user confirmation before running (e.g. a destructive command that IS
    /// on the allow list / permitted by the sandbox mode).
    NeedApproval(String),
    /// Unconditional block — never run this, regardless of user confirmation.
    Deny(String),
}

/// A tool invocation with all placeholders already resolved (real command,
/// real paths), ready for safety checks.
#[derive(Debug, Clone, Default)]
pub struct Invocation {
    pub tool_name: String,
    /// Resolved shell command, if this invocation runs one.
    pub command: Option<String>,
    /// Resolved filesystem paths this invocation touches.
    pub paths: Vec<String>,
}

impl Invocation {
    pub fn shell(tool_name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            command: Some(command.into()),
            paths: vec![],
        }
    }

    pub fn paths(tool_name: impl Into<String>, paths: Vec<String>) -> Self {
        Self {
            tool_name: tool_name.into(),
            command: None,
            paths,
        }
    }
}

/// Run every invocation through this before it executes.
pub fn check(inv: &Invocation, cfg: &Config, allowed: &[String]) -> Verdict {
    // `--dangerously-skip-permissions` / `/permissions skip`: bypass every
    // check below, including destructive-command and path-escape approval —
    // not just the allow-list Deny path, which was already implicitly
    // bypassed via `sandboxed`. This is the single funnel every invocation
    // passes through, so gating here covers the LLM tool-call path, skill
    // execution (inline and subagent), and interactive /skill and /command
    // runs uniformly.
    if cfg.skip_permissions {
        return Verdict::Allow;
    }

    for path in &inv.paths {
        if let Some(reason) = path_escape(&inv.tool_name, path, &cfg.work_dir) {
            return Verdict::NeedApproval(reason);
        }
    }

    if let Some(cmd) = &inv.command {
        let trimmed = cmd.trim();
        let sandboxed = cfg.sandbox_mode.allows_all() || cfg.skip_permissions;
        let wildcard = allowed.iter().any(|p| p == "*");

        if policy::has_chain_operators(trimmed) && !sandboxed && !wildcard {
            return Verdict::Deny(format!("chained command denied: {trimmed}"));
        }

        if !sandboxed && !policy::is_command_allowed(trimmed, allowed) {
            let first = trimmed.split_whitespace().next().unwrap_or(trimmed);
            return Verdict::Deny(format!(
                "not permitted: {trimmed:?} — use /allow {first} or /sandbox [permissive|docker|gvisor]"
            ));
        }

        if is_destructive_cmd(trimmed) {
            return Verdict::NeedApproval(format!("destructive command: {trimmed}"));
        }
    }

    Verdict::Allow
}

/// Returns `Some(reason)` if the (already-resolved) `path` would escape `work_dir`.
fn path_escape(tool_name: &str, path: &str, work_dir: &str) -> Option<String> {
    // Canonicalize work_dir *before* joining so both sides agree on symlinks
    // (e.g. macOS /tmp -> /private/tmp) — otherwise an in-bounds relative path
    // can spuriously look like an escape.
    let work_norm =
        std::fs::canonicalize(work_dir).unwrap_or_else(|_| normalize(Path::new(work_dir)));
    let resolved = executor::resolve_path(path, &work_norm.to_string_lossy());
    let resolved_norm = normalize(Path::new(&resolved));

    if resolved_norm.starts_with(&work_norm) || is_always_allowed_tmp(&resolved_norm, &work_norm) {
        None
    } else {
        Some(format!(
            "{tool_name}: path '{path}' resolves to '{}', outside the working directory '{}'",
            resolved_norm.display(),
            work_norm.display()
        ))
    }
}

/// System scratch directories are writable without approval when `work_dir`
/// itself sits outside them — they're throwaway space, not project state, so
/// flagging a write there as an "escape" is just noise. Covers `/tmp` directly
/// and its macOS symlink target `/private/tmp`, plus the process's actual
/// `std::env::temp_dir()` (which on macOS is a per-user `/var/folders/.../T`
/// path, not `/tmp` at all).
///
/// If `work_dir` is *itself* under one of these roots (a realistic setup for
/// sandboxed/ephemeral sessions), that root is excluded from the bypass —
/// otherwise it would swallow the very containment check it's meant to be a
/// narrow exception to, letting a write escape into a sibling directory (e.g.
/// another session's scratch space) with zero approval.
fn is_always_allowed_tmp(path: &Path, work_dir_norm: &Path) -> bool {
    let mut roots = vec![PathBuf::from("/tmp"), PathBuf::from("/private/tmp")];
    roots.push(std::env::temp_dir());
    roots.iter().any(|root| {
        let root_norm = std::fs::canonicalize(root).unwrap_or_else(|_| normalize(root));
        let in_root = path.starts_with(&root_norm) || path.starts_with(root);
        let work_dir_in_root =
            work_dir_norm.starts_with(&root_norm) || work_dir_norm.starts_with(root);
        in_root && !work_dir_in_root
    })
}

/// Lexically normalize a path (collapse `.` / `..`) without requiring it to exist.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

const DESTRUCTIVE_BINS: &[&str] = &[
    "rm", "rmdir", "dd", "mkfs", "fdisk", "shutdown", "reboot", "halt", "poweroff", "kill",
    "pkill", "killall",
];

/// Tokenized destructive-command classifier. Replaces the old substring match,
/// which never fired for `/bin/rm x` (absolute path) or `find . -delete`
/// (no literal "rm"), and which never ran at all for skill-routed commands.
pub fn is_destructive_cmd(cmd: &str) -> bool {
    if cmd
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .contains(":(){:|:&};:")
    {
        return true; // fork bomb
    }

    let lower = cmd.to_lowercase();
    for kw in ["drop table", "drop database", "truncate"] {
        if lower.contains(kw) {
            return true;
        }
    }

    for segment in split_segments(cmd) {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let Some(&first) = tokens.first() else {
            continue;
        };
        let argv0 = basename(first);

        if DESTRUCTIVE_BINS.contains(&argv0.as_str()) {
            return true;
        }

        match argv0.as_str() {
            "find" if tokens.contains(&"-delete") => return true,
            "xargs" => {
                // `foo | xargs rm` — the destructive binary is xargs's argument, not argv0.
                if let Some(inner) = tokens[1..].iter().find(|t| !t.starts_with('-')) {
                    if DESTRUCTIVE_BINS.contains(&basename(inner).as_str()) {
                        return true;
                    }
                }
            }
            "git" => {
                let rest = &tokens[1..];
                let sub = rest.first().copied().unwrap_or("");
                let has_force = rest.iter().any(|t| *t == "--force" || *t == "-f");
                if sub == "push" && has_force {
                    return true;
                }
                if sub == "reset" && rest.contains(&"--hard") {
                    return true;
                }
                if sub == "clean"
                    && (has_force || rest.iter().any(|t| t.starts_with('-') && t.contains('f')))
                {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn basename(argv0: &str) -> String {
    Path::new(argv0)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(argv0)
        .to_string()
}

/// Split a command into segments on `&&`, `||`, `;`, and `|`, respecting quotes,
/// so each pipeline/chain stage can be classified independently.
fn split_segments(cmd: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(c);
                i += 1;
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(c);
                i += 1;
            }
            '&' if !in_single && !in_double && chars.get(i + 1) == Some(&'&') => {
                segments.push(std::mem::take(&mut current));
                i += 2;
            }
            '|' if !in_single && !in_double => {
                segments.push(std::mem::take(&mut current));
                i += if chars.get(i + 1) == Some(&'|') { 2 } else { 1 };
            }
            ';' if !in_single && !in_double => {
                segments.push(std::mem::take(&mut current));
                i += 1;
            }
            _ => {
                current.push(c);
                i += 1;
            }
        }
    }
    segments.push(current);
    segments
}

// ── Skill validation ─────────────────────────────────────────────────────────
//
// Runs on every skill load/reload. Error-severity issues mean the skill is
// skipped (never silently — the caller surfaces every message); Warning-severity
// issues are reported but the skill still loads.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone)]
pub struct Issue {
    pub severity: Severity,
    pub message: String,
}

impl Issue {
    fn warn(msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: msg.into(),
        }
    }
    fn error(msg: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: msg.into(),
        }
    }
}

pub fn validate_skill(skill: &Skill) -> Vec<Issue> {
    let mut issues = Vec::new();

    if skill.name.trim().is_empty() {
        issues.push(Issue::error("skill name is empty"));
    }
    if skill.description.trim().is_empty() {
        issues.push(Issue::warn("description is empty"));
    }

    if skill.chunks.is_empty() && skill.body.trim().is_empty() {
        issues.push(Issue::error(
            "skill has neither a shell chunk nor a prompt body",
        ));
        return issues;
    }

    for (i, chunk) in skill.chunks.iter().enumerate() {
        let label = if skill.chunks.len() > 1 {
            format!("chunk {}", i + 1)
        } else {
            "command".to_string()
        };
        let cmd = &chunk.source;
        if cmd.trim().is_empty() {
            issues.push(Issue::error(format!("{label} is empty")));
            continue;
        }
        if !cmd.contains("{query}") {
            issues.push(Issue::warn(format!(
                "{label} never references {{query}} — the caller's input will be ignored"
            )));
        }
        // The exact bug this project's own built-in skills had: the placeholder
        // sitting inside the author's own quotes (wrapping it directly, or just
        // embedded in a larger quoted string) defeats the executor's
        // single-quoting and can reopen the injection it's meant to close.
        if placeholder_in_quotes(cmd, "{query}") {
            issues.push(Issue::error(format!(
                "{label}: {{query}} sits inside quotes — it is already substituted as \
                 a single shell-quoted word, so this breaks quoting; write `{{query}}` bare, \
                 not inside '...' or \"...\" (assign to a variable first if you need it in \
                 a larger quoted string, e.g. `Q={{query}}; ... \"$Q\"`)"
            )));
        }
        if let Some(bin) = first_binary(cmd) {
            if !bin.is_empty() && !binary_exists(&bin) {
                issues.push(Issue::warn(format!(
                    "{label}: binary '{bin}' not found in PATH"
                )));
            }
        }
        if shellcheck_available() {
            issues.extend(run_shellcheck(cmd));
        } else if is_destructive_cmd(cmd) {
            issues.push(Issue::warn(format!(
                "{label} matches a destructive pattern — running it will require interactive approval"
            )));
        }
    }

    if skill.chunks.is_empty() && !skill.body.contains("{input}") && !skill.body.contains("{query}")
    {
        issues.push(Issue::warn(
            "prompt body never references {input} or {query} — the caller's input will be ignored",
        ));
    }

    issues
}

/// True if `placeholder` occurs inside a single- or double-quoted span of `cmd`.
/// Operates on bytes throughout (placeholders are always ASCII) so it can't
/// panic on a mid-char slice for non-ASCII command text.
fn placeholder_in_quotes(cmd: &str, placeholder: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let bytes = cmd.as_bytes();
    let needle = placeholder.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if (in_single || in_double) && bytes[i..].starts_with(needle) {
            return true;
        }
        match bytes[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Best-effort argv0 of the first pipeline/chain segment, skipping leading
/// `VAR=value` shell assignments (e.g. `D={query}; find ...` -> "find").
fn first_binary(cmd: &str) -> Option<String> {
    let segment = cmd.split(['|', ';']).next().unwrap_or(cmd);
    let segment = segment.split("&&").next().unwrap_or(segment);
    let mut tokens = segment.split_whitespace();
    let mut tok = tokens.next()?;
    while tok.contains('=') && !tok.starts_with('/') && !tok.starts_with('.') {
        tok = tokens.next()?;
    }
    Some(tok.trim_matches(|c| c == '"' || c == '\'').to_string())
}

fn binary_exists(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let path = Path::new(name);
    if path.is_absolute() || name.starts_with("./") || name.starts_with("../") {
        return path.exists();
    }
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

fn shellcheck_available() -> bool {
    binary_exists("shellcheck")
}

fn run_shellcheck(cmd: &str) -> Vec<Issue> {
    use std::io::Write;
    let mut issues = Vec::new();
    let Ok(mut child) = std::process::Command::new("shellcheck")
        .args(["-s", "sh", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
    else {
        return issues;
    };
    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(cmd.as_bytes());
    }
    if let Ok(output) = child.wait_with_output() {
        let text = String::from_utf8_lossy(&output.stdout);
        for line in text.lines().filter(|l| l.contains("SC")) {
            issues.push(Issue::warn(line.to_string()));
        }
    }
    issues
}

// ── Startup diagnostics ──────────────────────────────────────────────────────
//
// Runs once in Engine::new. Purely informational — nothing here blocks
// startup; it exists because parse failures used to be eprintln!'d into a
// terminal the TUI had already taken over, making them invisible.

pub fn startup(
    cfg: &Config,
    marlin_dir: &Path,
    work_dir: &str,
    idx: Option<&crate::index::Index>,
) -> Vec<String> {
    let mut lines = Vec::new();

    match cfg.providers.get(&cfg.active_provider) {
        Some(p)
            if p.api_key.is_empty()
                && cfg.active_provider != "ollama"
                && cfg.active_provider != "custom" =>
        {
            lines.push(format!(
                "no API key set for active provider '{}' — use /key {} <key>",
                cfg.active_provider, cfg.active_provider
            ));
        }
        None => lines.push(format!(
            "active provider '{}' has no configuration",
            cfg.active_provider
        )),
        _ => {}
    }

    for bin in ["rg", "gh", "ast-compiler", "ast-harness"] {
        if !binary_exists(bin) {
            lines.push(format!(
                "'{bin}' not found in PATH — related features will be degraded"
            ));
        }
    }
    if !executor::detect_mxc() {
        lines.push(format!(
            "MXC binary '{}' not found — sandbox_mode=mxc will fail",
            executor::mxc_binary_name()
        ));
    }

    check_toml_file::<crate::config::ThemePalette>(
        &marlin_dir.join("theme.toml"),
        "theme.toml",
        &mut lines,
    );
    check_toml_file::<crate::config::LayoutConfig>(
        &marlin_dir.join("layout.toml"),
        "layout.toml",
        &mut lines,
    );
    // Skills aren't scanned here — .qmd isn't TOML, and skills::load_all already
    // runs validate_skill (parse errors + issues) as part of Engine::new/`/skill reload`.
    scan_toml_dir::<crate::tools::external::ExternalTool>(
        &crate::tools::external::tools_dir(marlin_dir),
        "tool",
        &mut lines,
    );
    scan_toml_dir::<crate::commands::UserCommand>(
        &crate::commands::commands_dir(marlin_dir),
        "command",
        &mut lines,
    );
    scan_toml_dir::<crate::providers::user_providers::UserProvider>(
        &crate::providers::user_providers::providers_dir(marlin_dir),
        "provider",
        &mut lines,
    );

    if let Some(idx) = idx {
        if let Some(newest) = newest_source_mtime(work_dir) {
            let built_at = std::time::UNIX_EPOCH
                + std::time::Duration::from_secs(idx.built_at.timestamp().max(0) as u64);
            if newest > built_at {
                lines.push(
                    "code index is stale — source files changed since the last /index build".into(),
                );
            }
        }
    }

    lines
}

fn check_toml_file<T: serde::de::DeserializeOwned>(
    path: &Path,
    label: &str,
    lines: &mut Vec<String>,
) {
    if !path.exists() {
        return;
    }
    match std::fs::read_to_string(path) {
        Ok(data) => {
            if let Err(e) = toml::from_str::<T>(&data) {
                lines.push(format!("✗ {label}: {e}"));
            }
        }
        Err(e) => lines.push(format!("✗ {label}: {e}")),
    }
}

fn scan_toml_dir<T: serde::de::DeserializeOwned>(dir: &Path, label: &str, lines: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(data) = std::fs::read_to_string(&path) {
            if let Err(e) = toml::from_str::<T>(&data) {
                lines.push(format!(
                    "✗ {label} {:?}: {e}",
                    path.file_name().unwrap_or_default()
                ));
            }
        }
    }
}

/// Newest mtime among source files under `work_dir`, skipping common junk dirs.
/// Bounded so a huge repo can't stall startup.
fn newest_source_mtime(work_dir: &str) -> Option<std::time::SystemTime> {
    fn walk(
        dir: &Path,
        depth: usize,
        budget: &mut usize,
        newest: &mut Option<std::time::SystemTime>,
    ) {
        if depth > 8 || *budget == 0 {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | "__pycache__"
            ) {
                continue;
            }
            if path.is_dir() {
                walk(&path, depth + 1, budget, newest);
            } else if let Ok(modified) = entry.metadata().and_then(|m| m.modified()) {
                if newest.map(|n| modified > n).unwrap_or(true) {
                    *newest = Some(modified);
                }
            }
        }
    }
    let mut newest = None;
    let mut budget = 20_000usize;
    walk(Path::new(work_dir), 0, &mut budget, &mut newest);
    newest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn cfg_with(work_dir: &str) -> Config {
        let mut c = Config::defaults();
        c.work_dir = work_dir.to_string();
        c
    }

    // ── is_destructive_cmd ───────────────────────────────────────────────────

    #[test]
    fn detects_absolute_path_rm() {
        assert!(is_destructive_cmd("/bin/rm -rf /tmp/x"));
    }

    #[test]
    fn detects_find_delete() {
        assert!(is_destructive_cmd("find . -name '*.log' -delete"));
    }

    #[test]
    fn detects_git_push_force() {
        assert!(is_destructive_cmd("git push --force origin main"));
        assert!(is_destructive_cmd("git push -f origin main"));
    }

    #[test]
    fn detects_git_reset_hard() {
        assert!(is_destructive_cmd("git reset --hard HEAD~1"));
    }

    #[test]
    fn detects_destructive_after_chain() {
        assert!(is_destructive_cmd("echo hi; rm -rf ~"));
        assert!(is_destructive_cmd("echo hi && rm -rf ~"));
        assert!(is_destructive_cmd("echo hi | xargs rm"));
    }

    #[test]
    fn benign_command_not_destructive() {
        assert!(!is_destructive_cmd("git status"));
        assert!(!is_destructive_cmd("ls -la"));
        assert!(!is_destructive_cmd("cargo build"));
    }

    // ── check(): the skill-injection bypass ──────────────────────────────────

    #[test]
    fn skill_command_with_empty_allow_list_is_denied() {
        // Default config: sandbox off, no allowed commands — this is the
        // exact scenario from the audit (explore skill, injected `;`).
        // The chain-operator gate fires before the allow-list check, but
        // either way the outcome is an unconditional Deny.
        let cfg = cfg_with("/tmp/marlin-test-workdir");
        let inv = Invocation::shell("run_skill", ".; touch /tmp/marlin_pwned");
        assert!(matches!(check(&inv, &cfg, &[]), Verdict::Deny(_)));
    }

    #[test]
    fn skill_command_without_chain_ops_denied_by_empty_allow_list() {
        let cfg = cfg_with("/tmp/marlin-test-workdir");
        let inv = Invocation::shell("run_skill", "touch /tmp/marlin_pwned");
        assert_eq!(
            check(&inv, &cfg, &[]),
            Verdict::Deny(
                "not permitted: \"touch /tmp/marlin_pwned\" — use /allow touch or /sandbox [permissive|docker|gvisor]".into()
            )
        );
    }

    #[test]
    fn skill_command_allowed_when_wildcard_present() {
        let cfg = cfg_with("/tmp/marlin-test-workdir");
        let inv = Invocation::shell("run_skill", "rg foo --heading");
        assert_eq!(check(&inv, &cfg, &["*".to_string()]), Verdict::Allow);
    }

    #[test]
    fn external_tool_command_denied_without_allow_list() {
        let cfg = cfg_with("/tmp/marlin-test-workdir");
        let inv = Invocation::shell("my_tool", "curl http://evil.example; rm -rf ~");
        matches!(check(&inv, &cfg, &[]), Verdict::Deny(_));
    }

    // ── check(): path containment ────────────────────────────────────────────

    #[test]
    fn path_within_work_dir_allowed() {
        let dir = std::env::temp_dir().join("marlin_preflight_test_within");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cfg_with(dir.to_str().unwrap());
        let inv = Invocation::paths("write_file", vec!["notes.txt".to_string()]);
        assert_eq!(check(&inv, &cfg, &[]), Verdict::Allow);
    }

    #[test]
    fn startup_does_not_panic_with_missing_binaries() {
        // This sandbox has no ast-compiler/ast-harness/gh/mxc installed — startup()
        // must degrade to diagnostic lines, never panic, regardless of environment.
        let dir = std::env::temp_dir().join("marlin_preflight_test_startup");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cfg_with(dir.to_str().unwrap());
        let lines = startup(&cfg, &dir, dir.to_str().unwrap(), None);
        // Just needs to return without panicking; content is environment-dependent.
        let _ = lines;
    }

    #[test]
    fn path_escaping_work_dir_needs_approval() {
        let dir = std::env::temp_dir().join("marlin_preflight_test_escape");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cfg_with(dir.to_str().unwrap());
        let inv = Invocation::paths("write_file", vec!["~/.ssh/authorized_keys".to_string()]);
        assert!(matches!(check(&inv, &cfg, &[]), Verdict::NeedApproval(_)));
    }

    #[test]
    fn tmp_path_outside_workdir_is_allowed_without_approval() {
        // work_dir doesn't need to exist on disk — canonicalize() falls back to
        // lexical normalize() when it can't resolve a path, which is all
        // path_escape needs. Using a fixed non-tmp path (rather than
        // env::temp_dir()) keeps this test meaningful in sandboxes where
        // TMPDIR is itself redirected under /tmp, which would otherwise make
        // work_dir and the tested path share a root by environmental accident.
        let cfg = cfg_with("/Users/nonexistent_marlin_test_project");
        // Absolute /tmp path, unrelated to work_dir — would normally be an escape.
        let inv = Invocation::paths(
            "write_file",
            vec!["/tmp/marlin_scratch_test.txt".to_string()],
        );
        assert_eq!(check(&inv, &cfg, &[]), Verdict::Allow);
    }

    #[test]
    fn tmp_sibling_path_still_needs_approval_when_workdir_is_itself_under_tmp() {
        let dir = std::env::temp_dir().join("marlin_preflight_test_workdir_itself_in_tmp");
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = cfg_with(dir.to_str().unwrap());
        // A sibling path under the SAME tmp root as work_dir, but outside work_dir —
        // must NOT be silently allowed just because it's "under tmp somewhere" (the
        // regression: work_dir itself lives under a sandboxed session's tmp root).
        let sibling = dir
            .parent()
            .unwrap()
            .join("marlin_preflight_test_sibling_secret.txt");
        let inv = Invocation::paths("write_file", vec![sibling.to_str().unwrap().to_string()]);
        assert!(matches!(check(&inv, &cfg, &[]), Verdict::NeedApproval(_)));
    }

    #[test]
    fn skip_permissions_bypasses_path_escape_and_destructive_command() {
        let dir = std::env::temp_dir().join("marlin_preflight_test_skip_permissions");
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = cfg_with(dir.to_str().unwrap());
        cfg.skip_permissions = true;

        let path_inv = Invocation::paths("write_file", vec!["~/.ssh/authorized_keys".to_string()]);
        assert_eq!(check(&path_inv, &cfg, &[]), Verdict::Allow);

        let cmd_inv = Invocation::shell("run_command", "rm -rf ~");
        assert_eq!(check(&cmd_inv, &cfg, &[]), Verdict::Allow);
    }

    // ── validate_skill ────────────────────────────────────────────────────────

    use crate::skills::{Chunk, SkillFormat};

    fn shell_skill(command: &str) -> Skill {
        Skill {
            name: "test_skill".into(),
            description: "a test skill".into(),
            triggers: vec![],
            body: String::new(),
            chunks: vec![Chunk {
                lang: "sh".into(),
                source: command.into(),
            }],
            format: SkillFormat::Qmd,
        }
    }

    fn prompt_skill(template: &str) -> Skill {
        Skill {
            name: "test_skill".into(),
            description: "a test skill".into(),
            triggers: vec![],
            body: template.into(),
            chunks: vec![],
            format: SkillFormat::Qmd,
        }
    }

    #[test]
    fn clean_shell_skill_has_no_errors() {
        let skill = shell_skill("rg {query} --color never --heading");
        assert!(validate_skill(&skill)
            .iter()
            .all(|i| i.severity != Severity::Error));
    }

    #[test]
    fn quoted_placeholder_is_an_error() {
        // This is the exact bug that shipped in the built-in `ripgrep` skill
        // before it was fixed — {query} wrapped in the author's own quotes
        // defeats the executor's single-quoting.
        let skill = shell_skill("rg '{query}' --color never --heading");
        let issues = validate_skill(&skill);
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
    }

    #[test]
    fn double_quoted_placeholder_is_an_error() {
        let skill = shell_skill(r#"curl "https://example.com/?q={query}""#);
        let issues = validate_skill(&skill);
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
    }

    #[test]
    fn empty_shell_command_is_an_error() {
        let skill = shell_skill("");
        let issues = validate_skill(&skill);
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
    }

    #[test]
    fn missing_query_placeholder_is_a_warning_not_an_error() {
        let skill = shell_skill("echo hello");
        let issues = validate_skill(&skill);
        assert!(issues.iter().any(|i| i.severity == Severity::Warning));
        assert!(issues.iter().all(|i| i.severity != Severity::Error));
    }

    #[test]
    fn empty_prompt_template_is_an_error() {
        let skill = prompt_skill("");
        let issues = validate_skill(&skill);
        assert!(issues.iter().any(|i| i.severity == Severity::Error));
    }

    #[test]
    fn clean_prompt_skill_has_no_errors() {
        let skill = prompt_skill("Summarize: {input}");
        assert!(validate_skill(&skill)
            .iter()
            .all(|i| i.severity != Severity::Error));
    }
}
