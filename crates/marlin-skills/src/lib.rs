pub mod daemon;
pub mod executor;
pub mod qmd;
pub mod skills;
pub mod suggest;

// Flattened re-exports so consumers can use `marlin_skills::load_all`,
// `marlin_skills::Skill`, `marlin_skills::executor`, etc. instead of reaching
// into the nested `skills` submodule.
pub use marlin_core::skill::{Chunk, Skill, SkillDef, SkillFormat};
pub use skills::*;
