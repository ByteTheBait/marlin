use anyhow::{anyhow, Result};

use super::{Skill, SkillKind};

pub struct SkillResult {
    pub output: String,
}

pub fn execute_shell(skill: &Skill, query: &str, work_dir: &str) -> Result<SkillResult> {
    if skill.run.kind != SkillKind::Shell {
        return Err(anyhow!("skill '{}' is not a shell skill", skill.name));
    }
    let cmd = skill.run.command.replace("{query}", query);
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(&cmd)
        .current_dir(work_dir)
        .output()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    Ok(SkillResult { output: text.trim().to_string() })
}

pub fn expand_prompt(skill: &Skill, input: &str) -> Result<String> {
    if skill.run.kind != SkillKind::Prompt {
        return Err(anyhow!("skill '{}' is not a prompt skill", skill.name));
    }
    Ok(skill.run.template
        .replace("{input}", input)
        .replace("{query}", input))
}
