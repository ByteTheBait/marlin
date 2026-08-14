use marlin_providers::Message;

// Thresholds in approximate tokens — scaled from the configured context budget
// (see `maybe_prune_history`). The mechanical fallback runs after the LLM
// summarizer when still over budget.
const KEEP_RECENT: usize = 6;
const TOOL_RESULT_CAP: usize = 300; // chars kept per tool result in compression
const CONTENT_TAIL: usize = 500; // chars kept for user/assistant messages

/// Approximate token count. Uses 4 chars/token (more accurate than /3 for mixed content).
/// Also adds per-message overhead for role tokens.
pub fn estimate_tokens(history: &[Message], system_prompt: &str) -> usize {
    let count = |s: &str| s.len().saturating_add(3) / 4;

    let sys = count(system_prompt);
    let hist: usize = history
        .iter()
        .map(|m| {
            let content = count(&m.content);
            let tools: usize = m
                .tool_calls
                .iter()
                .map(|tc| count(&tc.input).saturating_add(8))
                .sum();
            content + tools + 4 // 4-token overhead for role/message framing
        })
        .sum();

    sys + hist
}

/// Mechanical fallback compaction. Called after the LLM summarizer when still over budget.
/// Strategy: truncate tool results first (largest, lowest value), then user/assistant,
/// then drop oldest messages.
/// Returns (compressed, dropped) counts.
///
/// `budget` is the configured context budget (from `/budget`). Compaction starts
/// at 80% of budget and hard-drops at 95%, so raising the budget raises the
/// thresholds accordingly.
pub fn maybe_prune_history(history: &mut Vec<Message>, budget: usize) -> (usize, usize) {
    let compress_at = (budget as f64 * 0.80) as usize;
    let drop_at = (budget as f64 * 0.95) as usize;

    if estimate_tokens(history, "") < compress_at {
        return (0, 0);
    }
    if history.len() <= KEEP_RECENT {
        return (0, 0);
    }

    let split_idx = history.len() - KEEP_RECENT;
    let mut compressed = 0usize;

    // Pass 1: aggressively truncate tool results (usually the bulk of tokens)
    for msg in &mut history[0..split_idx] {
        if msg.role == "tool" && msg.content.len() > TOOL_RESULT_CAP {
            let head: String = msg.content.chars().take(TOOL_RESULT_CAP).collect();
            msg.content = format!("{head}\n[… truncated, was {} chars]", msg.content.len());
            compressed += 1;
        }
    }

    if estimate_tokens(history, "") < compress_at {
        return (compressed, 0);
    }

    // Pass 2: truncate long user/assistant messages to tail (preserve recent context)
    for msg in &mut history[0..split_idx] {
        if msg.role != "tool" && msg.content.len() > CONTENT_TAIL * 2 {
            let tail: String = msg
                .content
                .chars()
                .rev()
                .take(CONTENT_TAIL)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            msg.content = format!("[… condensed, was {} chars]\n…{}", msg.content.len(), tail);
            compressed += 1;
        }
    }

    // Pass 3: drop oldest messages (skip index 0 in case it's a compaction summary)
    let mut dropped = 0usize;
    while estimate_tokens(history, "") > drop_at && history.len() > KEEP_RECENT + 1 {
        history.remove(1);
        dropped += 1;
    }

    (compressed, dropped)
}
