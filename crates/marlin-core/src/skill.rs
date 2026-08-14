//! Skill types shared between the skills crate and the preflight crate
//! (which validates skills) and the engine (which executes them).

#[derive(Debug, Clone)]
pub struct Chunk {
    pub lang: String,
    pub source: String,
}

/// Which file format a skill was loaded from (or should round-trip back to).
/// `.toml` support is deprecated — kept for one release so `/skill migrate`
/// has something to migrate from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillFormat {
    Toml,
    Qmd,
}

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
    pub body: String,
    pub chunks: Vec<Chunk>,
    pub format: SkillFormat,
}

impl Skill {
    pub fn is_shell(&self) -> bool {
        !self.chunks.is_empty()
    }

    pub fn is_prompt(&self) -> bool {
        !self.body.trim().is_empty()
    }
}

/// Lightweight version sent to the TUI for suggestion matching.
#[derive(Debug, Clone)]
pub struct SkillDef {
    pub name: String,
    pub description: String,
    pub triggers: Vec<String>,
}

impl From<&Skill> for SkillDef {
    fn from(s: &Skill) -> Self {
        Self {
            name: s.name.clone(),
            description: s.description.clone(),
            triggers: s.triggers.clone(),
        }
    }
}
