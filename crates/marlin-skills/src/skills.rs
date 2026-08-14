use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Deserialize;

use crate::qmd;
use marlin_core::skill::{Chunk, Skill, SkillFormat};

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum LegacyKind {
    Shell,
    Prompt,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyRun {
    #[serde(rename = "type")]
    kind: LegacyKind,
    #[serde(default)]
    command: String,
    #[serde(default)]
    template: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyToml {
    name: String,
    description: String,
    #[serde(default)]
    triggers: Vec<String>,
    run: LegacyRun,
}

impl From<LegacyToml> for Skill {
    fn from(l: LegacyToml) -> Self {
        let (body, chunks) = match l.run.kind {
            LegacyKind::Shell => (
                String::new(),
                vec![Chunk {
                    lang: "sh".into(),
                    source: l.run.command,
                }],
            ),
            LegacyKind::Prompt => (l.run.template, vec![]),
        };
        Skill {
            name: l.name,
            description: l.description,
            triggers: l.triggers,
            body,
            chunks,
            format: marlin_core::skill::SkillFormat::Toml,
        }
    }
}

// ── I/O ──────────────────────────────────────────────────────────────────────

pub fn skills_dir(marlin_dir: &Path) -> PathBuf {
    let d = marlin_dir.join("skills");
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Load every `.qmd` and (deprecated) `.toml` skill in `~/.marlin/skills/`,
/// running each through `preflight::validate_skill`. A skill with an
/// Error-severity issue (broken placeholder quoting, empty command/template,
/// unparsable frontmatter, ...) is reported and skipped — never silently
/// dropped like a bare parse failure used to be. Returns the loaded skills
/// plus a diagnostic line per issue (including parse errors), for the caller
/// to surface once the TUI can display them.
pub fn load_all(marlin_dir: &Path) -> (Vec<Skill>, Vec<String>) {
    let dir = skills_dir(marlin_dir);
    let mut skills = Vec::new();
    let mut diagnostics = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (skills, diagnostics);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();

        let parsed = match ext {
            Some("qmd") => std::fs::read_to_string(&path)
                .ok()
                .map(|data| qmd::parse(&data).map_err(|e| format!("skill {file_name}: {e}"))),
            Some("toml") => std::fs::read_to_string(&path).ok().map(|data| {
                toml::from_str::<LegacyToml>(&data)
                    .map(Skill::from)
                    .map_err(|e| format!("skill {file_name}: parse error: {e}"))
            }),
            _ => None,
        };

        let Some(parsed) = parsed else { continue };
        match parsed {
            Err(msg) => diagnostics.push(msg),
            Ok(skill) => {
                let issues = marlin_preflight::preflight::validate_skill(&skill);
                let has_error = issues
                    .iter()
                    .any(|i| i.severity == marlin_preflight::preflight::Severity::Error);
                for issue in &issues {
                    diagnostics.push(format!("skill {file_name}: {}", issue.message));
                }
                if has_error {
                    diagnostics.push(format!("skill {file_name}: skipped due to the error above"));
                } else {
                    skills.push(skill);
                }
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    (skills, diagnostics)
}

/// Save a skill as `.qmd` — the current format. `.toml` is write-once (via the
/// legacy loader) and never produced by this path.
pub fn save_skill(marlin_dir: &Path, skill: &Skill) -> Result<PathBuf> {
    let dir = skills_dir(marlin_dir);
    let filename = format!("{}.qmd", skill.name.replace([' ', '/'], "_").to_lowercase());
    let path = dir.join(&filename);
    let data = qmd::to_string(skill)?;
    std::fs::write(&path, data)?;
    Ok(path)
}

/// Rewrite every `.toml` skill in `~/.marlin/skills/` to `.qmd`, removing the
/// original. Returns the number migrated.
pub fn migrate_all(marlin_dir: &Path) -> Result<usize> {
    let dir = skills_dir(marlin_dir);
    let mut migrated = 0;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(0);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        let Ok(data) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(skill) = toml::from_str::<LegacyToml>(&data).map(Skill::from) else {
            continue;
        };
        save_skill(marlin_dir, &skill)?;
        std::fs::remove_file(&path)?;
        migrated += 1;
    }
    Ok(migrated)
}

// ── Default skills ────────────────────────────────────────────────────────────

pub fn install_defaults(marlin_dir: &Path) {
    let dir = skills_dir(marlin_dir);
    for skill in default_skills() {
        let toml_twin = dir.join(format!("{}.toml", skill.name));
        let qmd_path = dir.join(format!("{}.qmd", skill.name));
        // A pre-existing .toml (from before this release, or user-authored)
        // wins — don't shadow it with a same-named .qmd default.
        if toml_twin.exists() || qmd_path.exists() {
            continue;
        }
        if let Ok(data) = qmd::to_string(&skill) {
            let _ = std::fs::write(qmd_path, data);
        }
    }
}

fn default_skills() -> Vec<Skill> {
    vec![
        Skill {
            name: "web_search".into(),
            description: "Search the web using DuckDuckGo (requires curl)".into(),
            triggers: vec![
                "search".into(), "look up".into(), "google".into(),
                "find online".into(), "web".into(), "internet".into(),
            ],
            body: String::new(),
            // {query} is substituted as a single shell-quoted word (see
            // skills::executor::resolve_chunks) and assigned to Q rather than
            // interpolated directly into the URL literal, so injected quotes
            // in the query can't break out of the double-quoted curl arg.
            //
            // --proto/--proto-redir pin both the initial request and any
            // redirect target to https, so a redirect can't be steered at
            // file:// or a plain-http internal/metadata address (most of
            // which don't speak TLS). --max-time and --max-filesize bound
            // how long we wait and how much a malicious/misbehaving server
            // can make curl buffer.
            chunks: vec![Chunk {
                lang: "sh".into(),
                source: concat!(
                    r#"Q={query}; curl -sL --proto '=https' --proto-redir '=https' "#,
                    r#"--max-time 15 --max-filesize 2000000 "#,
                    r#""https://html.duckduckgo.com/html/?q=$Q" "#,
                    r#"| grep -oP '(?<=class="result__snippet">)[^<]+' "#,
                    r#"| head -10 "#,
                    r#"| sed 's/&amp;/\&/g; s/&lt;/</g; s/&gt;/>/g; s/&#x27;/'"'"'/g'"#,
                ).into(),
            }],
            format: marlin_core::skill::SkillFormat::Qmd,
        },
        Skill {
            name: "ripgrep".into(),
            description: "Search code with ripgrep".into(),
            triggers: vec![
                "grep".into(), "rg".into(), "search code".into(),
                "find in files".into(), "find function".into(),
            ],
            body: String::new(),
            // {query} arrives already shell-quoted — leave it bare, no extra quotes.
            chunks: vec![Chunk {
                lang: "sh".into(),
                source: "rg {query} --color never --max-count 5 --heading".into(),
            }],
            format: SkillFormat::Qmd,
        },
        Skill {
            name: "explore".into(),
            description: "Show project file structure (excludes .git, node_modules, target)".into(),
            triggers: vec![
                "explore".into(), "structure".into(), "overview".into(),
                "list files".into(), "find files".into(), "what files".into(),
                "directory".into(), "tree".into(), "navigate".into(),
            ],
            body: String::new(),
            // {query} arrives already shell-quoted — leave it bare, no extra quotes.
            chunks: vec![Chunk {
                lang: "sh".into(),
                source: concat!(
                    r#"D={query}; find "${D:-.}" "#,
                    r#"-not -path '*/.git/*' "#,
                    r#"-not -path '*/node_modules/*' "#,
                    r#"-not -path '*/target/*' "#,
                    r#"-not -path '*/__pycache__/*' "#,
                    r#"-not -name '*.pyc' "#,
                    r#"| sort | head -80"#,
                ).into(),
            }],
            format: SkillFormat::Qmd,
        },
        Skill {
            name: "make_skill".into(),
            description: "Create a new Marlin skill file".into(),
            triggers: vec![
                "create skill".into(), "new skill".into(),
                "add skill".into(), "make skill".into(),
            ],
            // Deliberately describes the qmd fence syntax in prose rather than
            // demonstrating it with a real ```{sh} block — an actual fenced
            // example here would itself be parsed as this skill's own chunk.
            body: concat!(
                "Create a new Marlin skill file at ~/.marlin/skills/<name>.qmd\n\n",
                "The skill should do: {input}\n\n",
                "The file has three parts, in order:\n",
                "  1. YAML frontmatter between --- lines: name, description, triggers (a list).\n",
                "  2. Optional prose — shown to the model alongside chunk output.\n",
                "  3. Optional fenced shell chunk(s): a line reading exactly ", "```{sh}",
                " (or ```{bash}), then\n",
                "     the shell command(s), then a line reading exactly ", "```", ".\n\n",
                "{query} is substituted into a chunk as ONE shell-quoted word — leave it bare, do \n",
                "NOT wrap it in your own quotes (write `cmd {query}`, not `cmd '{query}'`). A skill \n",
                "with no fenced chunk is a prompt skill instead: its prose is expanded with {input} \n",
                "and fed back to the model, nothing is executed.\n\n",
                "Write the file using write_file.",
            ).into(),
            chunks: vec![],
            format: SkillFormat::Qmd,
        },
        Skill {
            name: "recent_commits".into(),
            description: "Show and summarize recent git commits (optionally filtered)".into(),
            triggers: vec![
                "recent commits".into(), "git log".into(), "git history".into(),
                "what changed".into(), "commit history".into(),
            ],
            // Demonstrates the combined chunk+prose behavior: the chunk runs
            // first, then this prose plus its output are fed to the model together.
            body: "The commit log above is from the working directory's git history. \
                Summarize the overall themes of recent work in a few sentences.".into(),
            chunks: vec![Chunk {
                lang: "sh".into(),
                // {query} arrives already shell-quoted — leave it bare, no extra quotes.
                // An empty query must NOT be passed as a bare positional arg (git treats
                // '' as an invalid revision); ${Q:+...} only adds the pathspec when non-empty.
                source: "Q={query}; git log --oneline -n 20 ${Q:+-- \"$Q\"}".into(),
            }],
            format: SkillFormat::Qmd,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor;

    #[test]
    fn default_skills_pass_validation_cleanly() {
        for skill in default_skills() {
            let issues = marlin_preflight::preflight::validate_skill(&skill);
            let errors: Vec<_> = issues
                .iter()
                .filter(|i| i.severity == marlin_preflight::preflight::Severity::Error)
                .collect();
            assert!(
                errors.is_empty(),
                "skill '{}' has validation errors: {errors:?}",
                skill.name
            );
        }
    }

    #[test]
    fn default_skills_round_trip_through_qmd() {
        for skill in default_skills() {
            let rendered = qmd::to_string(&skill).unwrap();
            let reparsed = qmd::parse(&rendered).unwrap();
            assert_eq!(reparsed.name, skill.name);
            assert_eq!(
                reparsed.chunks.len(),
                skill.chunks.len(),
                "skill {}",
                skill.name
            );
        }
    }

    #[test]
    fn legacy_toml_shell_skill_converts_correctly() {
        let toml_src = r#"
name = "old_skill"
description = "an old shell skill"
triggers = ["old"]

[run]
type = "shell"
command = "echo {query}"
"#;
        let legacy: LegacyToml = toml::from_str(toml_src).unwrap();
        let skill: Skill = legacy.into();
        assert!(skill.is_shell());
        assert!(!skill.is_prompt());
        assert_eq!(skill.chunks[0].source, "echo {query}");
    }

    #[test]
    fn legacy_toml_prompt_skill_converts_correctly() {
        let toml_src = r#"
name = "old_prompt"
description = "an old prompt skill"

[run]
type = "prompt"
template = "Do this: {input}"
"#;
        let legacy: LegacyToml = toml::from_str(toml_src).unwrap();
        let skill: Skill = legacy.into();
        assert!(!skill.is_shell());
        assert!(skill.is_prompt());
        assert_eq!(skill.body, "Do this: {input}");
    }

    #[test]
    fn broken_frontmatter_is_reported_and_skipped_not_dropped() {
        let dir = std::env::temp_dir().join("marlin_skills_test_broken_frontmatter");
        let skills_subdir = dir.join("skills");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&skills_subdir).unwrap();

        // Deliberate typo: "descriptio" instead of "description" — required field missing.
        std::fs::write(
            skills_subdir.join("broken.qmd"),
            "---\nname: broken_skill\ndescriptio: oops typo\n---\n\n```{sh}\necho hi\n```\n",
        )
        .unwrap();

        let (loaded, diagnostics) = load_all(&dir);
        assert!(
            loaded.iter().all(|s| s.name != "broken_skill"),
            "broken skill should not load"
        );
        assert!(
            diagnostics.iter().any(|d| d.contains("broken.qmd")),
            "broken skill's frontmatter error should be reported, not silently dropped: {diagnostics:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skill_migrate_round_trips_toml_to_qmd_with_identical_behavior() {
        let dir = std::env::temp_dir().join("marlin_skills_test_migrate");
        let skills_subdir = dir.join("skills");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&skills_subdir).unwrap();

        std::fs::write(
            skills_subdir.join("legacy_echo.toml"),
            "name = \"legacy_echo\"\ndescription = \"echoes the query\"\ntriggers = [\"echo\"]\n\n[run]\ntype = \"shell\"\ncommand = \"echo {query}\"\n",
        ).unwrap();

        let (before, _) = load_all(&dir);
        let before_skill = before
            .iter()
            .find(|s| s.name == "legacy_echo")
            .unwrap()
            .clone();

        let migrated = migrate_all(&dir).unwrap();
        assert_eq!(migrated, 1);
        assert!(
            !skills_subdir.join("legacy_echo.toml").exists(),
            "the .toml should be removed after migration"
        );
        assert!(
            skills_subdir.join("legacy_echo.qmd").exists(),
            "a .qmd twin should exist after migration"
        );

        let (after, _) = load_all(&dir);
        let after_skill = after.iter().find(|s| s.name == "legacy_echo").unwrap();

        assert_eq!(after_skill.name, before_skill.name);
        assert_eq!(after_skill.description, before_skill.description);
        assert_eq!(after_skill.triggers, before_skill.triggers);
        assert_eq!(after_skill.is_shell(), before_skill.is_shell());
        assert_eq!(
            executor::resolve_chunks(after_skill, "world").unwrap(),
            executor::resolve_chunks(&before_skill, "world").unwrap(),
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
