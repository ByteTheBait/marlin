//! Quarto-style `.qmd` skill format: YAML frontmatter for metadata, markdown
//! prose as the prompt body, fenced ` ```{sh}` / ` ```{bash}` chunks as the
//! executable shell. Replaces the old TOML `[run] type = "shell"|"prompt"`
//! split — a skill can now have chunks, body, or both (see `super::Skill`).
//!
//! Non-goal (deliberately out of scope): chunk options (`echo`, `eval`,
//! `label`) and output-chaining between chunks. Chunks execute in order;
//! nothing pipes forward from one chunk to the next.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use marlin_core::skill::{Chunk, Skill, SkillFormat};

#[derive(Debug, Serialize, Deserialize)]
struct Frontmatter {
    name: String,
    description: String,
    #[serde(default)]
    triggers: Vec<String>,
}

/// Parse a `.qmd` skill file's contents.
pub fn parse(data: &str) -> Result<Skill> {
    let data = data.strip_prefix('\u{feff}').unwrap_or(data); // tolerate a BOM
    let rest = data.trim_start_matches('\n');
    let rest = rest.strip_prefix("---").ok_or_else(|| {
        anyhow!("missing YAML frontmatter — a .qmd skill must start with a `---` block")
    })?;
    // Frontmatter ends at the next line that is exactly "---".
    let end = find_closing_fence(rest)
        .ok_or_else(|| anyhow!("frontmatter is never closed — expected a line with just `---`"))?;
    let yaml = &rest[..end];
    let body_source = &rest[end..].trim_start_matches('-').trim_start_matches('\n');

    let fm: Frontmatter = serde_yaml::from_str(yaml).map_err(|e| anyhow!("frontmatter: {e}"))?;

    let (body, chunks) = split_chunks(body_source);

    Ok(Skill {
        name: fm.name,
        description: fm.description,
        triggers: fm.triggers,
        body,
        chunks,
        format: SkillFormat::Qmd,
    })
}

/// Byte offset of the line `---` that closes the frontmatter block, i.e. the
/// index right after that line's trailing `\n` (or end of string).
fn find_closing_fence(s: &str) -> Option<usize> {
    let mut offset = 0;
    for line in s.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        if trimmed == "---" {
            return Some(offset);
        }
        offset += line.len();
    }
    None
}

/// Split markdown body into (prose, chunks). Prose is everything outside
/// fenced code blocks whose info string is `{sh}` or `{bash}`; those chunks'
/// bodies are collected separately, in document order.
fn split_chunks(body: &str) -> (String, Vec<Chunk>) {
    let mut prose = String::new();
    let mut chunks = Vec::new();
    let mut lines = body.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let lang = trimmed
            .strip_prefix("```{sh}")
            .map(|_| "sh")
            .or_else(|| trimmed.strip_prefix("```{bash}").map(|_| "bash"));

        let Some(lang) = lang else {
            prose.push_str(line);
            prose.push('\n');
            continue;
        };

        let mut source = String::new();
        for chunk_line in lines.by_ref() {
            if chunk_line.trim_end() == "```" {
                break;
            }
            source.push_str(chunk_line);
            source.push('\n');
        }
        chunks.push(Chunk {
            lang: lang.to_string(),
            source: source.trim_end_matches('\n').to_string(),
        });
    }

    (prose.trim().to_string(), chunks)
}

/// Render a skill back to `.qmd` text.
pub fn to_string(skill: &Skill) -> Result<String> {
    let fm = Frontmatter {
        name: skill.name.clone(),
        description: skill.description.clone(),
        triggers: skill.triggers.clone(),
    };
    let yaml = serde_yaml::to_string(&fm)?;

    let mut out = String::new();
    out.push_str("---\n");
    out.push_str(yaml.trim_end());
    out.push_str("\n---\n\n");

    if !skill.body.trim().is_empty() {
        out.push_str(skill.body.trim());
        out.push_str("\n\n");
    }

    for chunk in &skill.chunks {
        out.push_str(&format!("```{{{}}}\n", chunk.lang));
        out.push_str(chunk.source.trim_end_matches('\n'));
        out.push_str("\n```\n\n");
    }

    Ok(format!("{}\n", out.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shell_only_skill() {
        let src = "---\nname: gh_issues\ndescription: List open GitHub issues\ntriggers: [issues, gh]\n---\n\nList the open issues.\n\n```{sh}\ngh issue list --limit 20\n```\n";
        let skill = parse(src).unwrap();
        assert_eq!(skill.name, "gh_issues");
        assert_eq!(skill.description, "List open GitHub issues");
        assert_eq!(skill.triggers, vec!["issues", "gh"]);
        assert_eq!(skill.chunks.len(), 1);
        assert_eq!(skill.chunks[0].lang, "sh");
        assert_eq!(skill.chunks[0].source, "gh issue list --limit 20");
        assert_eq!(skill.body, "List the open issues.");
    }

    #[test]
    fn parses_prompt_only_skill() {
        let src =
            "---\nname: summarize\ndescription: Summarize input\n---\n\nSummarize this: {input}\n";
        let skill = parse(src).unwrap();
        assert!(skill.chunks.is_empty());
        assert_eq!(skill.body, "Summarize this: {input}");
    }

    #[test]
    fn parses_multiple_chunks_in_order() {
        let src = "---\nname: two_step\ndescription: two shell steps\n---\n\n```{sh}\necho one\n```\n\n```{bash}\necho two\n```\n";
        let skill = parse(src).unwrap();
        assert_eq!(skill.chunks.len(), 2);
        assert_eq!(skill.chunks[0].source, "echo one");
        assert_eq!(skill.chunks[1].source, "echo two");
        assert_eq!(skill.chunks[1].lang, "bash");
    }

    #[test]
    fn missing_frontmatter_errors() {
        assert!(parse("no frontmatter here\n").is_err());
    }

    #[test]
    fn unclosed_frontmatter_errors() {
        assert!(parse("---\nname: x\n").is_err());
    }

    #[test]
    fn round_trips_through_to_string_and_parse() {
        let original = Skill {
            name: "roundtrip".into(),
            description: "a round trip test".into(),
            triggers: vec!["rt".into()],
            body: "Some prose here.".into(),
            chunks: vec![Chunk {
                lang: "sh".into(),
                source: "echo {query}".into(),
            }],
            format: marlin_core::skill::SkillFormat::Qmd,
        };
        let rendered = to_string(&original).unwrap();
        let reparsed = parse(&rendered).unwrap();
        assert_eq!(reparsed.name, original.name);
        assert_eq!(reparsed.description, original.description);
        assert_eq!(reparsed.triggers, original.triggers);
        assert_eq!(reparsed.body, original.body);
        assert_eq!(reparsed.chunks.len(), original.chunks.len());
        assert_eq!(reparsed.chunks[0].source, original.chunks[0].source);
    }
}
