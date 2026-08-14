use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use marlin_snapshots::DiffLine;
use crate::styles::*;

/// Result of feeding a key event to the diff pane.
pub enum DiffOutcome {
    None,
    /// User closed the pane (Esc / q).
    Close,
}

/// Diff overlay opened with `/diff-mode` — current file content vs. its most
/// recent snapshot. Same "renders on top of chat" pattern as ViewerPane;
/// engine computes the diff once up front, this widget only renders it.
pub struct DiffPane {
    pub path: String,
    lines: Vec<DiffLine>,
    scroll: usize,
    viewport_height: usize,
}

impl DiffPane {
    pub fn new(path: String, lines: Vec<DiffLine>) -> Self {
        Self {
            path,
            lines,
            scroll: 0,
            viewport_height: 20,
        }
    }

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(self.viewport_height.max(1))
    }

    pub fn on_key(&mut self, key: KeyEvent) -> DiffOutcome {
        let step = self.viewport_height.max(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return DiffOutcome::Close,
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
        DiffOutcome::None
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let modal_w = area.width.saturating_sub(4).clamp(20, area.width.max(20));
        let modal_h = area.height.saturating_sub(2).clamp(6, area.height.max(6));
        let x = area.x + (area.width.saturating_sub(modal_w)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_h)) / 2;
        let modal = Rect {
            x,
            y,
            width: modal_w,
            height: modal_h,
        };

        Clear.render(modal, buf);

        let added = self
            .lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Added(_)))
            .count();
        let removed = self
            .lines
            .iter()
            .filter(|l| matches!(l, DiffLine::Removed(_)))
            .count();
        let hint = format!(" +{added} -{removed} — ↑/↓ PgUp/PgDn scroll, Esc/q close ");

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(col_cobalt()))
            .title(Span::styled(
                format!(" {} (vs. last snapshot) ", self.path),
                Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Line::from(Span::styled(hint, style_placeholder())).right_aligned());
        let inner = block.inner(modal);
        block.render(modal, buf);

        self.viewport_height = inner.height as usize;
        self.scroll = self.scroll.min(self.max_scroll());

        let visible: Vec<Line> = self
            .lines
            .iter()
            .skip(self.scroll)
            .take(inner.height as usize)
            .map(|l| match l {
                DiffLine::Context(s) => Line::from(vec![
                    Span::styled("  ", style_placeholder()),
                    Span::styled(s.clone(), style_inline_text()),
                ]),
                DiffLine::Added(s) => Line::from(vec![
                    Span::styled("+ ", style_success()),
                    Span::styled(s.clone(), style_success()),
                ]),
                DiffLine::Removed(s) => Line::from(vec![
                    Span::styled("- ", style_error()),
                    Span::styled(s.clone(), style_error()),
                ]),
            })
            .collect();

        Paragraph::new(visible).render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn pane_with_lines(n: usize, viewport_height: usize) -> DiffPane {
        let lines = (1..=n)
            .map(|i| DiffLine::Context(format!("line {i}")))
            .collect();
        let mut p = DiffPane::new("test.txt".into(), lines);
        p.viewport_height = viewport_height;
        p
    }

    #[test]
    fn esc_and_q_close() {
        let mut p = pane_with_lines(10, 5);
        assert!(matches!(p.on_key(key(KeyCode::Esc)), DiffOutcome::Close));
        assert!(matches!(
            p.on_key(key(KeyCode::Char('q'))),
            DiffOutcome::Close
        ));
    }

    #[test]
    fn scroll_clamps_to_max() {
        let mut p = pane_with_lines(10, 5);
        for _ in 0..20 {
            p.on_key(key(KeyCode::Down));
        }
        assert_eq!(p.scroll, 5);
        p.on_key(key(KeyCode::Home));
        assert_eq!(p.scroll, 0);
        p.on_key(key(KeyCode::End));
        assert_eq!(p.scroll, 5);
    }
}
