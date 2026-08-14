use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::config::AstMode;
use crate::tui::styles::*;

pub struct StatusBar {
    pub provider: String,
    pub model: String,
    pub mode: String,
    pub active_tool: String,
    pub streaming: bool,
    pub width: u16,
    pub ast_mode: AstMode,
    /// Current git branch of the work directory (None if not a git repo).
    pub git_branch: Option<String>,
    /// Set when the base prompt injection (system prompt + tool defs) exceeds
    /// its ~2k token target. Informational only — never blocks a request.
    pub prompt_budget_over: Option<usize>,
    /// Number of running background processes (started via bg_start).
    pub bg_count: usize,
    /// Frame counter for subtle animations (pulsing streaming indicator).
    pub frame: u64,
}

impl StatusBar {
    pub fn new(width: u16) -> Self {
        Self {
            provider: String::new(),
            model: String::new(),
            mode: "chat".into(),
            active_tool: String::new(),
            streaming: false,
            width,
            ast_mode: AstMode::Off,
            git_branch: None,
            prompt_budget_over: None,
            bg_count: 0,
            frame: 0,
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let mut left: Vec<Span> = Vec::new();

        left.push(Span::styled(
            format!(" {} ", self.mode.to_uppercase()),
            style_status_chip(),
        ));

        if !self.provider.is_empty() {
            left.push(Span::raw("  "));
            left.push(Span::styled(self.provider.clone(), style_status_provider()));
            left.push(Span::styled("  ·  ", style_system()));
            left.push(Span::styled(self.model.clone(), style_status_model()));
        }

        if let Some(branch) = &self.git_branch {
            left.push(Span::raw("    "));
            left.push(Span::styled(format!("  {branch}  "), style_status_git()));
        }

        if !self.active_tool.is_empty() {
            left.push(Span::raw("    "));
            let glyph = tool_glyph(&self.active_tool);
            left.push(Span::styled(format!("{glyph} "), style_status_tool()));
            left.push(Span::styled(
                self.active_tool.clone(),
                style_status_tool_name(),
            ));
        }

        match &self.ast_mode {
            AstMode::Off => {}
            AstMode::SExpr => {
                left.push(Span::raw("    "));
                left.push(Span::styled(" SEXPR ", style_status_ast_sexpr()));
            }
            AstMode::Harness => {
                left.push(Span::raw("    "));
                left.push(Span::styled(" HARNESS ", style_status_ast_harness()));
            }
        }

        if let Some(tokens) = self.prompt_budget_over {
            left.push(Span::raw("    "));
            left.push(Span::styled(
                format!(" ⚠ PROMPT {tokens}t "),
                style_status_budget_warn(),
            ));
        }

        if self.bg_count > 0 {
            left.push(Span::raw("    "));
            left.push(Span::styled(
                format!("  ⚙ {n} bg  ", n = self.bg_count),
                style_status_bg_chip(),
            ));
        }

        let (right_text, right_style) = if self.streaming {
            // Pulse the "streaming" indicator so it reads as alive.
            ("  streaming  ", style_status_streaming_pulse(self.frame))
        } else {
            ("             ", Style::default())
        };

        let left_len: usize = left.iter().map(|s| s.content.chars().count()).sum();
        let right_len = right_text.chars().count();
        let pad = (area.width as usize).saturating_sub(left_len + right_len);

        let mut all = left;
        all.push(Span::raw(" ".repeat(pad)));
        all.push(Span::styled(right_text, right_style));

        // Paragraph fills the entire row with the base bg before drawing spans,
        // so no cell escapes without the correct background color.
        Paragraph::new(Line::from(all))
            .style(style_status_bg())
            .render(area, buf);
    }
}
