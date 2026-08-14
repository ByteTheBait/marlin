use anyhow::{anyhow, Result};

use super::Skill;

/// Resolve every chunk's shell command in order, substituting `{query}` as a
/// single shell-quoted word in each. Skill authors must leave `{query}` bare
/// (not wrapped in their own quotes) — this is the only layer of quoting
/// applied, and it's what stops `{query}` from smuggling in `;`, `&&`,
/// backticks, etc. The caller is responsible for executing each result through
/// the main tool executor (which applies output truncation, logging, and
/// clean_env) — chunks don't chain, so run them independently in order.
pub fn resolve_chunks(skill: &Skill, query: &str) -> Result<Vec<String>> {
    if skill.chunks.is_empty() {
        return Err(anyhow!("skill '{}' has no shell chunks", skill.name));
    }
    let quoted = crate::tools::executor::shell_quote(query);
    Ok(skill
        .chunks
        .iter()
        .map(|c| c.source.replace("{query}", &quoted))
        .collect())
}

/// Expand a prompt skill's prose body, substituting `{input}`/`{query}` with
/// the caller's raw text (no shell-quoting — this is fed to the model, not a shell).
pub fn expand_prompt(skill: &Skill, input: &str) -> Result<String> {
    if !skill.is_prompt() {
        return Err(anyhow!("skill '{}' has no prompt body", skill.name));
    }
    Ok(skill
        .body
        .replace("{input}", input)
        .replace("{query}", input))
}
