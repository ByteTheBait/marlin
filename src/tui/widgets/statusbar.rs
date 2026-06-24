use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::tui::styles::*;

pub struct StatusBar {
    pub provider: String,
    pub model: String,
    pub mode: String,
    pub active_tool: String,
    pub streaming: bool,
    pub width: u16,
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
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        // Fill background
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_style(style_status_bg());
            buf[(x, area.top())].set_symbol(" ");
        }

        // Left side spans
        let mut left: Vec<Span> = Vec::new();

        // Mode chip
        left.push(Span::styled(
            format!(" {} ", self.mode.to_uppercase()),
            style_status_chip(),
        ));

        // Provider · model
        if !self.provider.is_empty() {
            left.push(Span::styled("  ", Style::default()));
            left.push(Span::styled(self.provider.clone(), style_status_provider()));
            left.push(Span::styled("  ·  ", style_system()));
            left.push(Span::styled(self.model.clone(), style_status_model()));
        }

        // Active tool
        if !self.active_tool.is_empty() {
            left.push(Span::styled("    ", Style::default()));
            left.push(Span::styled("@ ", style_status_tool()));
            left.push(Span::styled(self.active_tool.clone(), style_status_tool_name()));
        }

        // Right side: streaming indicator
        let right_text = if self.streaming {
            "  streaming  "
        } else {
            "             "
        };
        let right_style = if self.streaming {
            style_status_streaming()
        } else {
            Style::default()
        };

        // Compute padding between left and right
        let left_len: usize = left.iter().map(|s| s.content.chars().count()).sum();
        let right_len = right_text.chars().count();
        let pad = (self.width as usize).saturating_sub(left_len + right_len);

        let mut all: Vec<Span> = left;
        all.push(Span::raw(" ".repeat(pad)));
        all.push(Span::styled(right_text, right_style));

        // Write spans into buffer
        let y = area.top();
        let mut x = area.left();
        for span in &all {
            let available = area.right().saturating_sub(x) as usize;
            if available == 0 { break; }
            for ch in span.content.chars().take(available) {
                if x >= area.right() { break; }
                buf[(x, y)].set_symbol(&ch.to_string());
                buf[(x, y)].set_style(span.style);
                x += 1;
            }
        }
    }
}
