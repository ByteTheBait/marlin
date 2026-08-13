use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Widget},
};
use tui_textarea::TextArea;

use crate::tui::styles::*;

/// Result of feeding a key event to the editor pane.
pub enum EditorOutcome {
    None,
    /// User closed the pane without unsaved changes (or confirmed discarding
    /// them with a second Esc).
    Close,
    /// Ctrl+S — the pane's current buffer content, to be written through the
    /// engine's preflight funnel (see `Engine::save_editor_file`).
    Save(String),
}

/// Editable file pane opened with `/edit`. Text editing itself (cursor
/// movement, insert/delete, etc.) is delegated to tui-textarea — the same
/// crate the main chat input box uses — but rendering is hand-rolled rather
/// than using tui-textarea's own `Widget` impl: this project pins
/// `ratatui >= 0.30` while tui-textarea 0.7.0 pins `ratatui 0.29` as a
/// separate, type-incompatible dependency, so its `set_block`/`Widget::render`
/// API can't be called with our `Block`/`Style` types (the same reason
/// ChatView's own input box hand-rolls its rendering instead of using
/// tui-textarea's, which this widget's render loop mirrors).
/// Never writes to disk itself — Ctrl+S hands content to the engine and waits
/// for `UiUpdate::EditorSaved` (via `mark_saved`) to clear the dirty flag.
pub struct EditorPane {
    pub path: String,
    textarea: TextArea<'static>,
    /// Content as of the last successful save (or initial load) — compared
    /// against the live buffer to derive the dirty flag.
    original: String,
    /// Set after one Esc on a dirty buffer; a second Esc actually closes.
    /// Any other key clears it, so it can't be "armed" from an earlier edit.
    pending_discard: bool,
    /// Whether the file used CRLF line endings when opened — `str::lines()`
    /// strips `\r` from every line, so reconstructing with a plain `\n` would
    /// mark an untouched CRLF file dirty and silently convert it to LF on save.
    crlf: bool,
    /// Top visible line — adjusted each render to keep the cursor in view.
    scroll_row: usize,
    viewport_height: usize,
}

impl EditorPane {
    pub fn new(path: String, content: String) -> Self {
        let lines: Vec<String> = if content.is_empty() {
            vec![String::new()]
        } else {
            content.lines().map(str::to_string).collect()
        };
        Self {
            path,
            textarea: TextArea::new(lines),
            crlf: content.contains("\r\n"),
            original: content,
            pending_discard: false,
            scroll_row: 0,
            viewport_height: 20,
        }
    }

    /// Reconstructed buffer content, restoring the file's original line-ending
    /// style (CRLF vs LF) and trailing newline (tui-textarea's line list has
    /// neither) — otherwise an untouched CRLF file would compare unequal to
    /// `original` and get silently rewritten to LF on save.
    fn content(&self) -> String {
        let sep = if self.crlf { "\r\n" } else { "\n" };
        let joined = self.textarea.lines().join(sep);
        if self.original.ends_with('\n') && !joined.is_empty() {
            format!("{joined}{sep}")
        } else {
            joined
        }
    }

    fn is_dirty(&self) -> bool {
        self.content() != self.original
    }

    /// Called when `UiUpdate::EditorSaved` confirms the engine wrote this
    /// pane's last `Save` content to disk.
    pub fn mark_saved(&mut self) {
        self.original = self.content();
        self.pending_discard = false;
    }

    /// Insert pasted text (from a bracketed-paste event) into the buffer as a
    /// single unit, so newlines in the paste become line breaks rather than
    /// being interpreted as keys.
    pub fn paste(&mut self, text: &str) {
        self.pending_discard = false;
        self.textarea.insert_str(text);
    }

    pub fn on_key(&mut self, key: KeyEvent) -> EditorOutcome {
        if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.pending_discard = false;
            return EditorOutcome::Save(self.content());
        }

        if key.code == KeyCode::Esc {
            if self.is_dirty() && !self.pending_discard {
                self.pending_discard = true;
                return EditorOutcome::None;
            }
            return EditorOutcome::Close;
        }

        self.pending_discard = false;
        self.textarea.input(key);
        EditorOutcome::None
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let modal_w = area.width.saturating_sub(4).clamp(20, area.width.max(20));
        let modal_h = area.height.saturating_sub(2).clamp(6, area.height.max(6));
        let x = area.x + (area.width.saturating_sub(modal_w)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_h)) / 2;
        let modal = Rect { x, y, width: modal_w, height: modal_h };

        Clear.render(modal, buf);

        let dirty = self.is_dirty();
        let title = format!(" {}{} ", self.path, if dirty { " ●" } else { "" });
        let hint = if self.pending_discard {
            " unsaved changes — Esc again to discard "
        } else if dirty {
            " Ctrl+S save, Esc close "
        } else {
            " Ctrl+S save, Esc close (no changes) "
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(if dirty { col_amber() } else { col_cobalt() }))
            .title(Span::styled(title, Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD)))
            .title_bottom(Line::from(Span::styled(hint, style_placeholder())).right_aligned());
        let inner = block.inner(modal);
        block.render(modal, buf);

        self.viewport_height = inner.height as usize;

        let lines = self.textarea.lines();
        let (cursor_row, cursor_col) = self.textarea.cursor();

        // Keep the cursor's row inside the visible window.
        if cursor_row < self.scroll_row {
            self.scroll_row = cursor_row;
        } else if cursor_row >= self.scroll_row + self.viewport_height.max(1) {
            self.scroll_row = cursor_row + 1 - self.viewport_height.max(1);
        }

        let gutter_w = lines.len().to_string().len().max(3);
        let text_x = inner.x + gutter_w as u16 + 1;
        let text_w = inner.width.saturating_sub(gutter_w as u16 + 1) as usize;

        for (row_i, line) in lines.iter().enumerate().skip(self.scroll_row).take(inner.height as usize) {
            let row_y = inner.y + (row_i - self.scroll_row) as u16;

            let gutter = format!("{:>gutter_w$} ", row_i + 1);
            for (i, ch) in gutter.chars().enumerate() {
                let cx = inner.x + i as u16;
                if cx < inner.right() {
                    buf[(cx, row_y)].set_symbol(&ch.to_string());
                    buf[(cx, row_y)].set_style(style_placeholder());
                }
            }

            let chars: Vec<char> = line.chars().take(text_w).collect();
            for (i, ch) in chars.iter().enumerate() {
                let cx = text_x + i as u16;
                if cx < inner.right() {
                    buf[(cx, row_y)].set_symbol(&ch.to_string());
                    buf[(cx, row_y)].set_style(style_inline_text());
                }
            }

            // Cursor cell — reversed video, same idea as a terminal caret.
            if row_i == cursor_row {
                let cx = text_x + cursor_col.min(text_w) as u16;
                if cx < inner.right() {
                    if buf[(cx, row_y)].symbol().is_empty() {
                        buf[(cx, row_y)].set_symbol(" ");
                    }
                    let existing = buf[(cx, row_y)].style();
                    buf[(cx, row_y)].set_style(existing.add_modifier(Modifier::REVERSED));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_s() -> KeyEvent {
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
    }

    #[test]
    fn unmodified_buffer_closes_on_first_esc() {
        let mut p = EditorPane::new("f.txt".into(), "hello\n".into());
        assert!(matches!(p.on_key(key(KeyCode::Esc)), EditorOutcome::Close));
    }

    #[test]
    fn dirty_buffer_needs_two_esc_to_close() {
        let mut p = EditorPane::new("f.txt".into(), "hello\n".into());
        p.on_key(key(KeyCode::Char('x'))); // dirty the buffer
        assert!(p.is_dirty());
        assert!(matches!(p.on_key(key(KeyCode::Esc)), EditorOutcome::None));
        assert!(p.pending_discard);
        assert!(matches!(p.on_key(key(KeyCode::Esc)), EditorOutcome::Close));
    }

    #[test]
    fn any_key_after_pending_discard_disarms_it() {
        let mut p = EditorPane::new("f.txt".into(), "hello\n".into());
        p.on_key(key(KeyCode::Char('x')));
        p.on_key(key(KeyCode::Esc));
        assert!(p.pending_discard);
        p.on_key(key(KeyCode::Char('y')));
        assert!(!p.pending_discard);
    }

    #[test]
    fn ctrl_s_returns_current_content() {
        let mut p = EditorPane::new("f.txt".into(), "hello\n".into());
        p.on_key(key(KeyCode::Char('!')));
        match p.on_key(ctrl_s()) {
            EditorOutcome::Save(content) => assert_eq!(content, "!hello\n"),
            _ => panic!("expected Save"),
        }
    }

    #[test]
    fn mark_saved_clears_dirty_flag() {
        let mut p = EditorPane::new("f.txt".into(), "hello\n".into());
        p.on_key(key(KeyCode::Char('x')));
        assert!(p.is_dirty());
        p.mark_saved();
        assert!(!p.is_dirty());
        assert!(matches!(p.on_key(key(KeyCode::Esc)), EditorOutcome::Close));
    }

    #[test]
    fn empty_file_opens_with_one_blank_line() {
        let p = EditorPane::new("new.txt".into(), String::new());
        assert_eq!(p.content(), "");
        assert!(!p.is_dirty());
    }

    #[test]
    fn untouched_crlf_file_round_trips_and_is_not_dirty() {
        let p = EditorPane::new("f.txt".into(), "a\r\nb\r\nc\r\n".into());
        assert_eq!(p.content(), "a\r\nb\r\nc\r\n");
        assert!(!p.is_dirty());
    }

    #[test]
    fn edited_crlf_file_saves_with_crlf_preserved() {
        let mut p = EditorPane::new("f.txt".into(), "a\r\nb\r\n".into());
        p.on_key(key(KeyCode::Char('!')));
        match p.on_key(ctrl_s()) {
            EditorOutcome::Save(content) => assert_eq!(content, "!a\r\nb\r\n"),
            _ => panic!("expected Save"),
        }
    }
}
