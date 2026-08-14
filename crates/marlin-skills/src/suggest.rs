use marlin_core::skill::SkillDef;

#[derive(Debug, Clone)]
pub struct SkillMatch {
    pub name: String,
    pub description: String,
    pub score: usize,
}

/// Match skills against `input` using trigger keyword overlap.
/// Returns up to 3 matches, sorted by score descending.
pub fn match_skills(input: &str, skills: &[SkillDef]) -> Vec<SkillMatch> {
    if input.is_empty() {
        return vec![];
    }
    let lower = input.to_lowercase();

    let mut matches: Vec<SkillMatch> = skills
        .iter()
        .filter_map(|skill| {
            let score = skill
                .triggers
                .iter()
                .map(|trigger| {
                    let tl = trigger.to_lowercase();
                    if lower.contains(&tl) {
                        // multi-word triggers score higher
                        tl.split_whitespace().count() * 3
                    } else {
                        // individual word overlap
                        tl.split_whitespace()
                            .filter(|w| lower.split_whitespace().any(|iw| iw == *w))
                            .count()
                    }
                })
                .sum::<usize>();

            if score > 0 {
                Some(SkillMatch {
                    name: skill.name.clone(),
                    description: skill.description.clone(),
                    score,
                })
            } else {
                None
            }
        })
        .collect();

    matches.sort_by(|a, b| b.score.cmp(&a.score));
    matches.truncate(3);
    matches
}
