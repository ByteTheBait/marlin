use crate::providers::Message;

// Token-based thresholds (~3 chars/token for mixed code+prose)
const COMPRESS_AT_TOKENS: usize = 80_000;
const DROP_AT_TOKENS: usize    = 100_000;
const KEEP_RECENT: usize       = 6;
const TAIL_CHARS: usize        = 400;

pub fn estimate_tokens(history: &[Message], system_prompt: &str) -> usize {
    let chars: usize = system_prompt.len()
        + history.iter().map(|m| {
            m.content.len()
                + m.tool_calls.iter().map(|tc| tc.input.len()).sum::<usize>()
        }).sum::<usize>();
    chars / 3
}

// Returns (compressed_count, dropped_count)
pub fn maybe_prune_history(history: &mut Vec<Message>) -> (usize, usize) {
    if estimate_tokens(history, "") < COMPRESS_AT_TOKENS { return (0, 0); }
    if history.len() <= KEEP_RECENT { return (0, 0); }

    let split_idx = history.len() - KEEP_RECENT;
    let mut compressed = 0usize;

    for i in 0..split_idx {
        let msg = &mut history[i];
        if msg.content.len() > TAIL_CHARS * 2 {
            let tail: String = msg.content.chars().rev().take(TAIL_CHARS).collect::<String>()
                .chars().rev().collect();
            msg.content = format!(
                "[Marlin: condensed older context — {} chars → tail]\n…{}",
                msg.content.len(), tail
            );
            compressed += 1;
        }
    }

    let mut dropped = 0usize;
    while estimate_tokens(history, "") > DROP_AT_TOKENS && history.len() > KEEP_RECENT + 1 {
        history.remove(1);
        dropped += 1;
    }

    (compressed, dropped)
}
