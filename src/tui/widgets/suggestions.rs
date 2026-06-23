use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget},
};

use crate::tui::styles::*;

#[derive(Clone)]
pub struct CmdDef {
    pub cmd: String,
    pub args: String,
    pub desc: String,
}

pub fn all_commands() -> Vec<CmdDef> {
    let raw = [
        ("/help",      "",                  "show all commands"),
        ("/clear",     "",                  "clear chat history"),
        ("/provider",  "<name>",            "switch provider"),
        ("/p",         "<name>",            "switch provider (short)"),
        ("/model",     "<name>",            "switch model"),
        ("/m",         "<name>",            "switch model (short)"),
        ("/providers", "",                  "list all providers"),
        ("/models",    "",                  "list models for current provider"),
        ("/key",       "<provider> <key>",  "set API key"),
        ("/endpoint",  "<provider> <url>",  "set API endpoint"),
        ("/system",    "<prompt>",          "set additional system prompt"),
        ("/sys",       "<prompt>",          "set system prompt (short)"),
        ("/tokens",    "[n]",               "get/set max output tokens"),
        ("/attach",    "<file>",            "attach file to next message"),
        ("/a",         "<file>",            "attach file (short)"),
        ("/detach",    "[file]",            "remove attachment(s)"),
        ("/exec",        "<cmd>",             "run shell command (must be /allow-ed)"),
        ("/allow",       "<prefix>",          "allow a shell command prefix"),
        ("/sandbox",     "[on|off]",          "allow all commands autonomously (persists)"),
        ("/permissions", "[skip|require]",    "skip or require permission checks (persists)"),
        ("/index",     "[status]",          "build TF-IDF search index"),
        ("/search",    "<query>",           "search the codebase index"),
        ("/revert",    "<file> [n]",        "show or restore file snapshots"),
        ("/resume",    "",                  "resume the most recent session"),
        ("/history",   "[n|clear]",         "list or load saved sessions"),
        ("/cat",       "<file>",            "print file contents"),
        ("/ls",        "[dir]",             "list directory"),
        ("/cd",        "<dir>",             "change working directory"),
        ("/pwd",       "",                  "show working directory"),
    ];
    raw.iter().map(|(c, a, d)| CmdDef {
        cmd: c.to_string(),
        args: a.to_string(),
        desc: d.to_string(),
    }).collect()
}

pub fn filter_suggestions<'a>(typed: &str, defs: &'a [CmdDef]) -> Vec<&'a CmdDef> {
    if !typed.starts_with('/') || typed.contains(' ') { return vec![]; }
    defs.iter()
        .filter(|d| d.cmd.starts_with(typed))
        .take(6)
        .collect()
}

pub fn tab_complete(typed: &str, suggestions: &[&CmdDef]) -> Option<String> {
    if suggestions.is_empty() { return None; }
    if suggestions.len() == 1 { return Some(suggestions[0].cmd.clone()); }
    let mut prefix = suggestions[0].cmd.clone();
    for s in &suggestions[1..] {
        while !s.cmd.starts_with(&prefix) {
            prefix.pop();
            if prefix.is_empty() { return None; }
        }
    }
    if prefix.len() > typed.len() { Some(prefix) } else { None }
}

pub struct SuggestionPanel<'a> {
    pub suggestions: &'a [&'a CmdDef],
    pub typed: &'a str,
    pub width: u16,
    pub frame: u64,
    pub streaming: bool,
}

// Bubble animation: dots wave in and out over a 4-phase cycle (~200 ms/phase at 60 fps)
fn bubble_dots(frame: u64) -> &'static str {
    match (frame / 12) % 6 {
        0 => "  ",
        1 => "o ",
        2 => "o o",
        3 => "o o o",
        4 => "o o",
        5 => "o ",
        _ => "  ",
    }
}

impl<'a> Widget for SuggestionPanel<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if self.streaming && self.suggestions.is_empty() {
            // ── Typing bubble ─────────────────────────────────────────────────
            let border_style = Style::default().fg(Color::Rgb(25, 45, 80));
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(border_style);

            let inner = block.inner(area);
            block.render(area, buf);

            if inner.height > 0 {
                let dots = bubble_dots(self.frame);
                let y = inner.y + inner.height / 2;
                let line = Line::from(vec![
                    Span::styled("  marlin  ", style_assistant_label()),
                    Span::styled(dots, Style::default().fg(Color::Rgb(0, 155, 155))),
                ]);
                Paragraph::new(line).render(
                    Rect { y, height: 1, ..inner },
                    buf,
                );
            }
        } else if !self.suggestions.is_empty() {
            // ── Slash-command completions ──────────────────────────────────────
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(Color::Rgb(30, 55, 100)));

            let inner = block.inner(area);
            block.render(area, buf);

            let lines: Vec<Line> = self.suggestions.iter().map(|s| {
                let exact = s.cmd == self.typed;
                let cmd_style = if exact {
                    Style::default().fg(COL_AQUA).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(COL_COBALT)
                };

                let raw_len = s.cmd.len()
                    + if s.args.is_empty() { 0 } else { 2 + s.args.len() };
                let pad = 30usize.saturating_sub(raw_len).max(2);

                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(s.cmd.clone(), cmd_style),
                ];
                if !s.args.is_empty() {
                    spans.push(Span::raw("  "));
                    spans.push(Span::styled(
                        s.args.clone(),
                        Style::default().fg(COL_STEEL),
                    ));
                }
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(
                    s.desc.clone(),
                    Style::default().fg(Color::Rgb(65, 85, 105)),
                ));
                Line::from(spans)
            }).collect();

            Paragraph::new(lines).render(inner, buf);
        }
    }
}
