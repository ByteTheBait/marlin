use anyhow::{anyhow, Result};

use super::{Skill, SkillKind};

/// Resolve the shell command for a shell skill, substituting `{query}`.
/// The caller is responsible for executing it through the main tool executor
/// (which applies output truncation, logging, and clean_env).
pub fn skill_command(skill: &Skill, query: &str) -> Result<String> {
    if skill.run.kind != SkillKind::Shell {
        return Err(anyhow!("skill '{}' is not a shell skill", skill.name));
    }
    Ok(skill.run.command.replace("{query}", query))
}

pub fn expand_prompt(skill: &Skill, input: &str) -> Result<String> {
    if skill.run.kind != SkillKind::Prompt {
        return Err(anyhow!("skill '{}' is not a prompt skill", skill.name));
    }
    Ok(skill.run.template
        .replace("{input}", input)
        .replace("{query}", input))
}
