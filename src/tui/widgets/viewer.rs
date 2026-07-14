use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use crate::tui::styles::*;

/// Result of feeding a key event to the viewer pane.
pub enum ViewerOutcome {
    None,
    /// User closed the pane (Esc / q).
    Close,
}

/// Read-only file preview overlay opened with `/view` or `/open`. Renders as
/// a bordered pane on top of chat, scrollable with arrow/vim keys and
/// PageUp/PageDown; the engine reads the file once up front, this widget
/// never touches disk itself.
pub struct ViewerPane {
    pub path: String,
    lines: Vec<String>,
    scroll: usize,
    /// Height of the last rendered inner area — `on_key` uses this (from the
    /// previous frame) to size Page/End jumps, same pattern as ChatView's
    /// own viewport scrolling.
    viewport_height: usize,
}

impl ViewerPane {
    pub fn new(path: String, content: String) -> Self {
        let lines: Vec<String> = content.lines().map(str::to_string).collect();
        Self { path, lines, scroll: 0, viewport_height: 20 }
    }

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.viewport_height.max(1))
    }

    pub fn on_key(&mut self, key: KeyEvent) -> ViewerOutcome {
        let step = self.viewport_height.max(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return ViewerOutcome::Close,
            KeyCode::Up | KeyCode::Char('k') => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = (self.scroll + 1).min(self.max_scroll());
            }
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(step),
            KeyCode::PageDown => self.scroll = (self.scroll + step).min(self.max_scroll()),
            KeyCode::Home => self.scroll = 0,
            KeyCode::End => self.scroll = self.max_scroll(),
            _ => {}
        }
        ViewerOutcome::None
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let modal_w = area.width.saturating_sub(4).clamp(20, area.width.max(20));
        let modal_h = area.height.saturating_sub(2).clamp(6, area.height.max(6));
        let x = area.x + (area.width.saturating_sub(modal_w)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_h)) / 2;
        let modal = Rect { x, y, width: modal_w, height: modal_h };

        Clear.render(modal, buf);

        let total = self.lines.len().max(1);
        let hint = format!(
            " {}/{total} — ↑/↓ PgUp/PgDn Home/End scroll, Esc/q close ",
            (self.scroll + 1).min(total)
        );

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(col_cobalt()))
            .title(Span::styled(
                format!(" {} ", self.path),
                Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Line::from(Span::styled(hint, style_placeholder())).right_aligned());
        let inner = block.inner(modal);
        block.render(modal, buf);

        self.viewport_height = inner.height as usize;
        self.scroll = self.scroll.min(self.max_scroll());

        let gutter_w = self.lines.len().to_string().len().max(3);
        let visible_lines: Vec<Line> = self.lines.iter().enumerate()
            .skip(self.scroll)
            .take(inner.height as usize)
            .map(|(i, l)| Line::from(vec![
                Span::styled(format!("{:>gutter_w$} ", i + 1), style_placeholder()),
                Span::styled(l.clone(), style_inline_text()),
            ]))
            .collect();

        Paragraph::new(visible_lines).render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn pane_with_lines(n: usize, viewport_height: usize) -> ViewerPane {
        let content = (1..=n).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let mut p = ViewerPane::new("test.txt".into(), content);
        p.viewport_height = viewport_height;
        p
    }

    #[test]
    fn esc_and_q_close() {
        let mut p = pane_with_lines(10, 5);
        assert!(matches!(p.on_key(key(KeyCode::Esc)), ViewerOutcome::Close));
        assert!(matches!(p.on_key(key(KeyCode::Char('q'))), ViewerOutcome::Close));
    }

    #[test]
    fn scroll_does_not_go_negative() {
        let mut p = pane_with_lines(10, 5);
        p.on_key(key(KeyCode::Up));
        assert_eq!(p.scroll, 0);
        p.on_key(key(KeyCode::PageUp));
        assert_eq!(p.scroll, 0);
    }

    #[test]
    fn scroll_clamps_to_max() {
        let mut p = pane_with_lines(10, 5);
        // max_scroll = 10 - 5 = 5
        for _ in 0..20 {
            p.on_key(key(KeyCode::Down));
        }
        assert_eq!(p.scroll, 5);

        p.scroll = 0;
        p.on_key(key(KeyCode::End));
        assert_eq!(p.scroll, 5);

        p.on_key(key(KeyCode::Home));
        assert_eq!(p.scroll, 0);
    }

    #[test]
    fn short_file_has_zero_max_scroll() {
        let mut p = pane_with_lines(3, 20);
        p.on_key(key(KeyCode::PageDown));
        assert_eq!(p.scroll, 0);
    }
}
