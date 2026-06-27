pub mod daemon;
pub mod executor;
pub mod suggest;

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── Skill types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SkillKind {
    Shell,
    Prompt,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRun {
    #[serde(rename = "type")]
    pub kind: SkillKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub command: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub triggers: Vec<String>,
    pub run: SkillRun,
}

/// Lightweight version sent to the TUI for suggestion matching.
#[derive(Debug, Clone)]
pub struct SkillDef {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    #[allow(dead_code)]
    pub kind: SkillKind,
}

impl From<&Skill> for SkillDef {
    fn from(s: &Skill) -> Self {
        Self {
            name: s.name.clone(),
            description: s.description.clone(),
            triggers: s.triggers.clone(),
            kind: s.run.kind.clone(),
        }
    }
}

// ── I/O ──────────────────────────────────────────────────────────────────────

pub fn skills_dir(marlin_dir: &Path) -> PathBuf {
    let d = marlin_dir.join("skills");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load_all(marlin_dir: &Path) -> Vec<Skill> {
    let dir = skills_dir(marlin_dir);
    let mut skills = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else { return skills };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue;
        }
        if let Ok(data) = std::fs::read_to_string(&path) {
            match toml::from_str::<Skill>(&data) {
                Ok(skill) => skills.push(skill),
                Err(e) => eprintln!("skills: parse error in {:?}: {e}", path.file_name()),
            }
        }
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

pub fn save_skill(marlin_dir: &Path, skill: &Skill) -> Result<PathBuf> {
    let dir = skills_dir(marlin_dir);
    let filename = format!("{}.toml", skill.name.replace([' ', '/'], "_").to_lowercase());
    let path = dir.join(&filename);
    let data = toml::to_string_pretty(skill)?;
    std::fs::write(&path, data)?;
    Ok(path)
}

// ── Default skills ────────────────────────────────────────────────────────────

pub fn install_defaults(marlin_dir: &Path) {
    let dir = skills_dir(marlin_dir);
    let defaults = default_skills();
    for skill in &defaults {
        let path = dir.join(format!("{}.toml", skill.name));
        if !path.exists() {
            if let Ok(data) = toml::to_string_pretty(skill) {
                let _ = std::fs::write(path, data);
            }
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
            run: SkillRun {
                kind: SkillKind::Shell,
                command: concat!(
                    r#"curl -sL "https://html.duckduckgo.com/html/?q={query}" "#,
                    r#"| grep -oP '(?<=class="result__snippet">)[^<]+' "#,
                    r#"| head -10 "#,
                    r#"| sed 's/&amp;/\&/g; s/&lt;/</g; s/&gt;/>/g; s/&#x27;/'"'"'/g'"#,
                ).into(),
                template: String::new(),
            },
        },
        Skill {
            name: "ripgrep".into(),
            description: "Search code with ripgrep".into(),
            triggers: vec![
                "grep".into(), "rg".into(), "search code".into(),
                "find in files".into(), "find function".into(),
            ],
            run: SkillRun {
                kind: SkillKind::Shell,
                command: "rg '{query}' --color never --max-count 5 --heading".into(),
                template: String::new(),
            },
        },
        Skill {
            name: "explore".into(),
            description: "Show project file structure (excludes .git, node_modules, target)".into(),
            triggers: vec![
                "explore".into(), "structure".into(), "overview".into(),
                "list files".into(), "find files".into(), "what files".into(),
                "directory".into(), "tree".into(), "navigate".into(),
            ],
            run: SkillRun {
                kind: SkillKind::Shell,
                command: concat!(
                    r#"D="{query}"; find "${D:-.}" "#,
                    r#"-not -path '*/.git/*' "#,
                    r#"-not -path '*/node_modules/*' "#,
                    r#"-not -path '*/target/*' "#,
                    r#"-not -path '*/__pycache__/*' "#,
                    r#"-not -name '*.pyc' "#,
                    r#"| sort | head -80"#,
                ).into(),
                template: String::new(),
            },
        },
        Skill {
            name: "make_skill".into(),
            description: "Create a new Marlin skill file".into(),
            triggers: vec![
                "create skill".into(), "new skill".into(),
                "add skill".into(), "make skill".into(),
            ],
            run: SkillRun {
                kind: SkillKind::Prompt,
                command: String::new(),
                template: concat!(
                    "Create a new Marlin skill file at ~/.marlin/skills/<name>.toml\n\n",
                    "The skill should do: {input}\n\n",
                    "Use this TOML format:\n",
                    "```toml\n",
                    "name = \"skill_name\"\n",
                    "description = \"What it does\"\n",
                    "triggers = [\"keyword1\", \"keyword2\"]\n\n",
                    "[run]\n",
                    "type = \"shell\"  # or \"prompt\"\n",
                    "command = \"cmd {query}\"  # for shell\n",
                    "# template = \"AI prompt with {input}\"  # for prompt\n",
                    "```\n\n",
                    "Write the file using write_file.",
                ).into(),
            },
        },
    ]
}
