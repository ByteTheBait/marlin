use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::tui::styles::*;

// ── Word wrap helper ─────────────────────────────────────────────────────────

pub(super) fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    if text.chars().count() <= max_width {
        return vec![text.to_string()];
    }
    let mut result: Vec<String> = Vec::new();
    let mut line = String::new();

    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if line.is_empty() {
            if word_len >= max_width {
                let mut chars = word.chars();
                loop {
                    let chunk: String = chars.by_ref().take(max_width).collect();
                    if chunk.is_empty() { break; }
                    if chunk.chars().count() < max_width {
                        line = chunk;
                        break;
                    }
                    result.push(chunk);
                }
            } else {
                line = word.to_string();
            }
        } else if line.chars().count() + 1 + word_len <= max_width {
            line.push(' ');
            line.push_str(word);
        } else {
            result.push(std::mem::take(&mut line));
            if word_len >= max_width {
                let mut chars = word.chars();
                loop {
                    let chunk: String = chars.by_ref().take(max_width).collect();
                    if chunk.is_empty() { break; }
                    if chunk.chars().count() < max_width {
                        line = chunk;
                        break;
                    }
                    result.push(chunk);
                }
            } else {
                line = word.to_string();
            }
        }
    }
    if !line.is_empty() {
        result.push(line);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

// ── <think> block handling ────────────────────────────────────────────────────
//
// Reasoning models (DeepSeek-R1, QwQ, gpt-oss, etc. served through
// OpenAI-compatible endpoints) emit their chain-of-thought inline in the
// content stream, wrapped in a bare `<think>...</think>` tag rather than a
// separate field. Split those out so they render dimmed/italic instead of
// as literal markdown text — including the case where the closing tag
// hasn't arrived yet, since this runs against the live streaming buffer.

enum Segment {
    Normal(String),
    Thinking(String),
}

fn split_think_segments(text: &str) -> Vec<Segment> {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut segments = Vec::new();
    let mut rest = text;
    while let Some(pos) = rest.find(OPEN) {
        if pos > 0 {
            segments.push(Segment::Normal(rest[..pos].to_string()));
        }
        rest = &rest[pos + OPEN.len()..];
        match rest.find(CLOSE) {
            Some(end) => {
                segments.push(Segment::Thinking(rest[..end].to_string()));
                rest = &rest[end + CLOSE.len()..];
            }
            None => {
                segments.push(Segment::Thinking(rest.to_string()));
                return segments;
            }
        }
    }
    if !rest.is_empty() {
        segments.push(Segment::Normal(rest.to_string()));
    }
    segments
}

fn render_thinking(text: &str, width: usize) -> Vec<Line<'static>> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![Line::from(Span::styled("✻ thinking", style_thinking()))];
    for raw_line in trimmed.lines() {
        if raw_line.trim().is_empty() {
            continue;
        }
        for wrapped in word_wrap(raw_line, width) {
            lines.push(Line::from(Span::styled(wrapped, style_thinking())));
        }
    }
    lines
}

// ── Markdown renderer (lightweight inline) ───────────────────────────────────

pub(super) fn render_markdown(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    for segment in split_think_segments(text) {
        match segment {
            Segment::Thinking(s) => lines.extend(render_thinking(&s, width)),
            Segment::Normal(s) => lines.extend(render_markdown_body(&s, width)),
        }
    }
    lines
}

fn render_markdown_body(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        if raw_line.starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                lines.push(Line::from(Span::styled(raw_line.to_string(), style_system())));
            } else {
                lines.push(Line::from(Span::styled("```".to_string(), style_system())));
            }
            continue;
        }
        if in_code_block {
            // Wrap code lines by character (don't reformat whitespace)
            let chars: Vec<char> = raw_line.chars().collect();
            if width == 0 || chars.len() <= width {
                lines.push(Line::from(Span::styled(raw_line.to_string(), style_code_block())));
            } else {
                for chunk in chars.chunks(width.max(1)) {
                    lines.push(Line::from(Span::styled(chunk.iter().collect::<String>(), style_code_block())));
                }
            }
            continue;
        }
        if let Some(rest) = raw_line.strip_prefix("# ") {
            for wrapped in word_wrap(rest, width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(col_user()).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )));
            }
            continue;
        }
        if let Some(rest) = raw_line.strip_prefix("## ") {
            for wrapped in word_wrap(rest, width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD),
                )));
            }
            continue;
        }
        if let Some(rest) = raw_line.strip_prefix("### ") {
            for wrapped in word_wrap(rest, width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(col_steel()).add_modifier(Modifier::BOLD),
                )));
            }
            continue;
        }
        // Inline spans (bold, italic, code) — wrap then parse each chunk
        if width > 0 && raw_line.chars().count() > width {
            for wrapped in word_wrap(raw_line, width) {
                lines.push(parse_inline(&wrapped));
            }
        } else {
            lines.push(parse_inline(raw_line));
        }
    }

    lines
}

/// Byte offset of the next `*` that stands alone (not part of a `**` pair).
/// `*` is ASCII, so every offset returned is a valid `str` char-boundary
/// regardless of surrounding UTF-8 text.
fn find_single_star(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'*' {
            if i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                i += 2; // skip the "**" pair entirely
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

enum Marker { Code, Bold, Italic }

fn parse_inline(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut rest = line;

    while !rest.is_empty() {
        // Whichever marker (code/bold/italic) opens earliest in the
        // remaining text wins — a line like "`code` then **bold**" must
        // resolve the backtick first even though bold is checked below it.
        let candidates = [
            rest.find('`').map(|p| (p, Marker::Code)),
            rest.find("**").map(|p| (p, Marker::Bold)),
            find_single_star(rest).map(|p| (p, Marker::Italic)),
        ];
        let earliest = candidates.into_iter().flatten().min_by_key(|(p, _)| *p);

        let Some((pos, marker)) = earliest else {
            spans.push(Span::styled(rest.to_string(), style_inline_text()));
            break;
        };

        if pos > 0 {
            spans.push(Span::styled(rest[..pos].to_string(), style_inline_text()));
        }

        match marker {
            Marker::Code => {
                rest = &rest[pos + 1..];
                match rest.find('`') {
                    Some(end) => {
                        spans.push(Span::styled(rest[..end].to_string(), style_inline_code()));
                        rest = &rest[end + 1..];
                    }
                    None => {
                        spans.push(Span::styled("`".to_string(), style_inline_text()));
                    }
                }
            }
            Marker::Bold => {
                rest = &rest[pos + 2..];
                match rest.find("**") {
                    Some(end) => {
                        spans.push(Span::styled(rest[..end].to_string(), style_inline_bold()));
                        rest = &rest[end + 2..];
                    }
                    None => {
                        spans.push(Span::styled("**".to_string(), style_inline_text()));
                    }
                }
            }
            Marker::Italic => {
                rest = &rest[pos + 1..];
                match find_single_star(rest) {
                    Some(end) => {
                        spans.push(Span::styled(rest[..end].to_string(), style_inline_italic()));
                        rest = &rest[end + 1..];
                    }
                    None => {
                        spans.push(Span::styled("*".to_string(), style_inline_text()));
                    }
                }
            }
        }
    }

    if spans.is_empty() {
        Line::from(Span::styled(line.to_string(), style_inline_text()))
    } else {
        Line::from(spans)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line<'static>]) -> String {
        lines.iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn renders_closed_think_block_before_the_answer() {
        let out = render_markdown("<think>reasoning here</think>the answer", 80);
        let text = plain(&out);
        assert!(text.contains("✻ thinking"));
        assert!(text.contains("reasoning here"));
        assert!(text.contains("the answer"));
        // the reasoning line should come before the answer line
        assert!(text.find("reasoning here").unwrap() < text.find("the answer").unwrap());
    }

    #[test]
    fn renders_unterminated_think_block_while_streaming() {
        let out = render_markdown("<think>still reasoning", 80);
        let text = plain(&out);
        assert!(text.contains("✻ thinking"));
        assert!(text.contains("still reasoning"));
    }

    #[test]
    fn text_without_think_tags_is_unaffected() {
        let out = render_markdown("just a normal answer", 80);
        let text = plain(&out);
        assert_eq!(text, "just a normal answer");
    }

    #[test]
    fn empty_think_block_produces_no_lines() {
        let out = render_markdown("<think></think>answer", 80);
        let text = plain(&out);
        assert_eq!(text, "answer");
    }
}
