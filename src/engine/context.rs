use crate::providers::Message;

const COMPRESS_THRESHOLD: usize = 320_000;
const MAX_CHARS: usize = 400_000;
const MSG_SIZE_LIMIT: usize = 2_000;
const TAIL_KEEP: usize = 500;
const KEEP_RECENT: usize = 6;

pub fn history_char_len(history: &[Message]) -> usize {
    history.iter().map(|m| {
        m.content.len() + m.tool_calls.iter().map(|tc| tc.input.len()).sum::<usize>()
    }).sum()
}

pub fn estimate_tokens(history: &[Message], system_prompt: &str) -> usize {
    let mut n = system_prompt.len() / 4;
    for m in history {
        n += m.content.len() / 4;
        for tc in &m.tool_calls { n += tc.input.len() / 4; }
    }
    n
}

// Returns (compressed_count, dropped_count)
pub fn maybe_prune_history(history: &mut Vec<Message>) -> (usize, usize) {
    if history_char_len(history) <= COMPRESS_THRESHOLD { return (0, 0); }
    if history.len() <= KEEP_RECENT { return (0, 0); }

    let split_idx = history.len() - KEEP_RECENT;
    let mut compressed = 0usize;

    for i in 0..split_idx {
        let msg = &mut history[i];
        if msg.content.len() > MSG_SIZE_LIMIT {
            let saved = msg.content.len();
            let tail: String = msg.content.chars().rev().take(TAIL_KEEP).collect::<String>()
                .chars().rev().collect();
            msg.content = format!(
                "[Marlin: truncated {} chars of older context to protect token budget]\n…{}",
                saved - TAIL_KEEP, tail
            );
            compressed += 1;
        }
    }

    let mut dropped = 0usize;
    let mut split = split_idx;
    while history_char_len(history) > MAX_CHARS && split > 1 {
        history.remove(1);
        split -= 1;
        dropped += 1;
    }

    (compressed, dropped)
}
