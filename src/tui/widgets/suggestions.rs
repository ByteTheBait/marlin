use ratatui::{
    buffer::Buffer,
    layout::Rect,
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
        ("/config",    "",                  "interactive settings menu"),
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
        ("/budget",    "[n]",               "get/set context budget (sidebar meter ceiling)"),
        ("/attach",    "<file>",            "attach file to next message"),
        ("/a",         "<file>",            "attach file (short)"),
        ("/detach",    "[file]",            "remove attachment(s)"),
        ("/exec",        "<cmd>",             "run shell command (must be /allow-ed)"),
        ("/allow",       "<prefix>",          "allow a shell command prefix"),
        ("/sandbox",     "[off|permissive|mxc]", "command isolation: off=require /allow, permissive=allow all, mxc=MS eXecution Containers"),
        ("/permissions", "[skip|require]",    "skip or require permission checks (persists)"),
        ("/verify",      "[cmd|off]",         "run command after every file edit (Write-Test-Fix)"),
        ("/ast",         "[off|sexpr|harness]", "AST context mode: sexpr=S-expr reads, harness=JSON surgery"),
        ("/clean-env",   "[on|off]",          "strip subprocess environment for isolation"),
        ("/theme",       "[dark|light|<name>]", "switch theme or apply a named theme (~/.marlin/themes/)"),
        ("/index",     "[status]",          "build TF-IDF search index"),
        ("/search",    "<query>",           "search the codebase index"),
        ("/revert",    "<file> [n]",        "show or restore file snapshots"),
        ("/resume",    "",                  "resume the most recent session"),
        ("/history",   "[n|clear]",         "list or load saved sessions"),
        ("/cat",       "<file>",            "print file contents"),
        ("/view",      "<file>",            "open a scrollable read-only pane for a file"),
        ("/open",      "<file>",            "alias for /view"),
        ("/diff-mode", "<file>",            "show current file vs. its most recent snapshot"),
        ("/edit",      "<file>",            "open an editable pane (Ctrl+S save, Esc close)"),
        ("/ls",        "[dir]",             "list directory"),
        ("/cd",        "<dir>",             "change working directory"),
        ("/pwd",       "",                  "show working directory"),
        ("/skill",     "[list|run|new|suggest|reload]", "manage and run skills"),
        ("/tiers",     "[on|off]",          "model tier routing with backup fallback"),
        ("/animate",   "[on|off]",          "toggle typewriter animation for AI responses"),
        ("/command",   "[list|new|reload]",  "manage user-defined slash commands (~/.marlin/commands/)"),
        ("/tool",      "[list|new|reload]",  "manage user-defined LLM tools (~/.marlin/tools/)"),
        ("/mcp",       "[list|new|reload]",  "manage MCP server connections (~/.marlin/mcp/)"),
        ("/provider",  "[list|new <name>|<name>]", "list/create user providers or switch provider"),
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

/// A lightweight skill hint shown in the suggestion panel.
#[derive(Clone, Debug)]
pub struct SkillHint {
    pub name: String,
    pub description: String,
}

pub struct SuggestionPanel<'a> {
    pub suggestions: &'a [&'a CmdDef],
    pub typed: &'a str,
    #[allow(dead_code)] // set by caller for future layout use
    pub width: u16,
    pub frame: u64,
    pub streaming: bool,
    /// Skill matches to show when the user is typing a non-slash message.
    pub skill_hints: &'a [SkillHint],
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
        if self.streaming && self.suggestions.is_empty() && self.skill_hints.is_empty() {
            // ── Typing bubble ─────────────────────────────────────────────────
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(style_bubble_border());

            let inner = block.inner(area);
            block.render(area, buf);

            if inner.height > 0 {
                let dots = bubble_dots(self.frame);
                let y = inner.y + inner.height / 2;
                let line = Line::from(vec![
                    Span::styled("  marlin  ", style_assistant_label()),
                    Span::styled(dots, style_bubble_dots()),
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
                .border_style(style_suggestion_border());

            let inner = block.inner(area);
            block.render(area, buf);

            let lines: Vec<Line> = self.suggestions.iter().map(|s| {
                let exact = s.cmd == self.typed;
                let cmd_style = if exact {
                    style_suggestion_cmd_exact()
                } else {
                    style_suggestion_cmd()
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
                    spans.push(Span::styled(s.args.clone(), style_suggestion_args()));
                }
                spans.push(Span::raw(" ".repeat(pad)));
                spans.push(Span::styled(s.desc.clone(), style_suggestion_desc()));
                Line::from(spans)
            }).collect();

            Paragraph::new(lines).render(inner, buf);
        } else if !self.skill_hints.is_empty() {
            // ── Skill hints ────────────────────────────────────────────────────
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(style_suggestion_border())
                .title(Span::styled(" skills ", style_suggestion_desc()));

            let inner = block.inner(area);
            block.render(area, buf);

            let lines: Vec<Line> = self.skill_hints.iter().map(|h| {
                let pad = 20usize.saturating_sub(h.name.len()).max(2);
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(h.name.clone(), style_suggestion_cmd()),
                    Span::raw(" ".repeat(pad)),
                    Span::styled(h.description.clone(), style_suggestion_desc()),
                ])
            }).collect();

            Paragraph::new(lines).render(inner, buf);
        }
    }
}
