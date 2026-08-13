use chrono::Local;
use tui_scrollview::ScrollViewState;
use tui_textarea::TextArea;

use crate::engine::Action;
use crate::tui::widgets::config_menu::ConfigMenuOutcome;
use crate::tui::widgets::diff::DiffOutcome;
use crate::tui::widgets::editor::EditorOutcome;
use crate::tui::widgets::suggestions::{CmdDef, filter_suggestions, tab_complete};
use crate::tui::widgets::viewer::ViewerOutcome;

use super::entry::{ChatEntry, EntryRole};
use super::state::ChatView;

impl ChatView {
    /// Shared by the ↑/↓ viewport-scroll fallback (see `on_key`) and mouse
    /// wheel scrolling — `steps` units in one direction, updating `at_bottom`
    /// the same way either input source needs it to.
    fn scroll_viewport(&mut self, up: bool, steps: u16) {
        if up {
            self.at_bottom = false;
            for _ in 0..steps {
                self.scroll_state.scroll_up();
            }
        } else {
            for _ in 0..steps {
                self.scroll_state.scroll_down();
            }
            let max = self.content_height.saturating_sub(self.viewport_height);
            self.at_bottom = self.scroll_state.offset().y >= max;
            if self.at_bottom {
                self.new_content_arrived = false;
            }
        }
    }

    /// Mouse wheel scroll. Overlay panes (config menu, /view, /diff-mode,
    /// /edit) and the approval modal already intercept ↑/↓ themselves inside
    /// `on_key` before input-history logic ever runs, so a wheel notch is
    /// safe to forward there as a synthetic key. In the base chat state it
    /// scrolls the viewport directly instead, since a literal ↑/↓ *keypress*
    /// there can mean "navigate input history" depending on cursor/history
    /// state — a wheel notch should always mean "scroll", never that.
    pub fn on_mouse_scroll(&mut self, up: bool) -> Option<Action> {
        let overlay_open = self.approval_pending.is_some()
            || self.config_menu.is_some()
            || self.viewer.is_some()
            || self.diff_pane.is_some()
            || self.editor.is_some();

        if overlay_open {
            let code = if up { crossterm::event::KeyCode::Up } else { crossterm::event::KeyCode::Down };
            return self.on_key(crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE));
        }

        self.scroll_viewport(up, 3);
        None
    }

    /// Handle a bracketed-paste event. The terminal delivers pasted text as a
    /// single `Event::Paste` (thanks to `EnableBracketedPaste`), so a paste
    /// containing newlines is inserted as one unit instead of each Enter being
    /// interpreted as "send message". Routes to whichever textarea is active:
    /// the ask_user modal, the /edit pane, or the main chat input.
    pub fn on_paste(&mut self, text: &str) {
        // ask_user modal — paste into the answer box.
        if self.ask_pending.is_some() {
            self.textarea.insert_str(text);
            return;
        }

        // /config menu — paste into the active text field (API key, new-provider
        // wizard). Ignored when not editing so it can't leak into the chat input
        // behind the menu.
        if let Some(menu) = &mut self.config_menu {
            menu.paste(text);
            return;
        }

        // /edit pane — paste into the file buffer.
        if let Some(editor) = &mut self.editor {
            editor.paste(text);
            return;
        }

        // Main chat input.
        self.textarea.insert_str(text);
        self.update_suggestions();
    }

    fn update_suggestions(&mut self) {
        let val = self.textarea.lines().first().cloned().unwrap_or_default();
        let defs = &self.suggestions_defs;
        let suggs = filter_suggestions(&val, defs);
        self.suggestions = suggs.iter().map(|s| {
            defs.iter().position(|d| std::ptr::eq(d, *s)).unwrap_or(0)
        }).collect();

        self.skill_hints.clear();
    }

    pub fn on_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        use crossterm::event::{KeyCode, KeyModifiers};

        // ask_user modal intercepts all input — Enter submits the typed answer,
        // Esc cancels.
        if self.ask_pending.is_some() {
            match key.code {
                KeyCode::Enter => {
                    let answer: String = self.textarea.lines().join("\n").trim().to_string();
                    self.ask_pending = None;
                    self.textarea = TextArea::default();
                    self.textarea.set_placeholder_text("Message Marlin... (Enter to send, Ctrl+J for newline)");
                    return Some(Action::UserAnswer(answer));
                }
                KeyCode::Esc => {
                    self.ask_pending = None;
                    self.textarea = TextArea::default();
                    self.textarea.set_placeholder_text("Message Marlin... (Enter to send, Ctrl+J for newline)");
                    return Some(Action::CancelStream);
                }
                _ => {
                    // Let the user type their answer in the input box.
                    self.textarea.input(key);
                    return None;
                }
            }
        }

        // Approval modal intercepts all input
        if self.approval_pending.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                    self.approval_pending = None;
                    self.add_system("Command approved.");
                    return Some(Action::Approve);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.approval_pending = None;
                    self.add_system("Command denied.");
                    return Some(Action::Deny);
                }
                _ => return None,
            }
        }

        // Config menu intercepts all input while open
        if let Some(menu) = &mut self.config_menu {
            match menu.on_key(key) {
                ConfigMenuOutcome::Close => {
                    self.config_menu = None;
                }
                ConfigMenuOutcome::Set { key, value } => {
                    if key == "animate" {
                        // TUI-local toggle — no engine round-trip needed.
                        self.typewriter_enabled = value == "on";
                        self.typewriter_pos = 0;
                        return None;
                    }
                    return Some(Action::ConfigSet { key: key.to_string(), value });
                }
                ConfigMenuOutcome::None => {}
            }
            return None;
        }

        // Viewer pane intercepts all input while open
        if let Some(viewer) = &mut self.viewer {
            if let ViewerOutcome::Close = viewer.on_key(key) {
                self.viewer = None;
            }
            return None;
        }

        // Diff pane intercepts all input while open
        if let Some(diff) = &mut self.diff_pane {
            if let DiffOutcome::Close = diff.on_key(key) {
                self.diff_pane = None;
            }
            return None;
        }

        // Editor pane intercepts all input while open
        if let Some(editor) = &mut self.editor {
            match editor.on_key(key) {
                EditorOutcome::Close => self.editor = None,
                EditorOutcome::Save(content) => {
                    let path = editor.path.clone();
                    return Some(Action::SaveEditorFile { path, content });
                }
                EditorOutcome::None => {}
            }
            return None;
        }

        // Ctrl+C / Esc — cancel a running stream. Pressed again with nothing
        // running, it quits (same as Ctrl+Q) instead of doing nothing; any
        // other key disarms that "press again to quit" state so it only
        // fires on two presses in a row, not two presses minutes apart.
        let is_cancel_key = key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL));
        if is_cancel_key {
            if self.rate_limited || self.streaming {
                self.rate_limited = false;
                self.streaming = false;
                self.stream_buf.clear();
                self.typewriter_pos = 0;
                self.active_goal.clear();
                self.tool_iterations = 0;
                self.quit_armed = false;
                self.add_system("Cancelled.");
                return Some(Action::CancelStream);
            }
            if self.quit_armed {
                return Some(Action::Quit);
            }
            self.quit_armed = true;
            self.add_system("Nothing to cancel — press Ctrl+C or Esc again to quit.");
            return None;
        }
        self.quit_armed = false;

        // Tab autocomplete
        if key.code == KeyCode::Tab {
            let val = self.textarea.lines().first().cloned().unwrap_or_default();
            let defs = &self.suggestions_defs;
            let suggs: Vec<&CmdDef> = self.suggestions.iter()
                .map(|&i| &defs[i]).collect();
            if let Some(completed) = tab_complete(&val, &suggs) {
                self.textarea = TextArea::default();
                self.textarea.insert_str(&completed);
                self.update_suggestions();
            }
            return None;
        }

        // Up/Down — input history
        if key.code == KeyCode::Up
            && !self.textarea.lines().iter().any(|l| l.contains('\n'))
            && (self.history_idx + 1) < self.input_history.len() as i32
        {
            if self.history_idx == -1 {
                self.history_draft = self.textarea.lines().join("\n");
            }
            self.history_idx += 1;
            let text = self.input_history[self.history_idx as usize].clone();
            self.textarea = TextArea::default();
            self.textarea.insert_str(&text);
            self.update_suggestions();
            return None;
        }
        // Falls through to viewport scroll when history is exhausted.

        if key.code == KeyCode::Down && self.history_idx >= 0 {
            self.history_idx -= 1;
            let text = if self.history_idx < 0 {
                std::mem::take(&mut self.history_draft)
            } else {
                self.input_history[self.history_idx as usize].clone()
            };
            self.textarea = TextArea::default();
            self.textarea.insert_str(&text);
            self.update_suggestions();
            return None;
        }

        // Viewport scroll
        if key.code == KeyCode::Up {
            self.scroll_viewport(true, 3);
            return None;
        }
        if key.code == KeyCode::Down {
            self.scroll_viewport(false, 3);
            return None;
        }
        if key.code == KeyCode::PageUp {
            self.at_bottom = false;
            for _ in 0..self.viewport_height {
                self.scroll_state.scroll_up();
            }
            return None;
        }
        if key.code == KeyCode::PageDown {
            for _ in 0..self.viewport_height {
                self.scroll_state.scroll_down();
            }
            let max = self.content_height.saturating_sub(self.viewport_height);
            self.at_bottom = self.scroll_state.offset().y >= max;
            if self.at_bottom {
                self.new_content_arrived = false;
            }
            return None;
        }
        if key.code == KeyCode::End {
            self.at_bottom = true;
            self.new_content_arrived = false;
            return None; // render_viewport will pin to bottom
        }
        if key.code == KeyCode::Home {
            self.at_bottom = false;
            self.scroll_state = ScrollViewState::default();
            return None;
        }

        // Enter to send (not Alt+Enter or Ctrl+J)
        if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::ALT) {
            let input: String = self.textarea.lines().join("\n").trim().to_string();
            if input.is_empty() { return None; }

            self.textarea = TextArea::default();
            self.textarea.set_placeholder_text("Message Marlin... (Enter to send, Ctrl+J for newline)");
            self.suggestions.clear();
            self.history_idx = -1;
            self.history_draft.clear();
            self.at_bottom = true;
            self.new_content_arrived = false;

            // Add to local input history (display only)
            if self.input_history.first().map(String::as_str) != Some(&input) {
                self.input_history.insert(0, input.clone());
            }

            // /animate is handled locally — no engine roundtrip needed
            if input == "/animate" || input.starts_with("/animate ") {
                let arg = input["animate".len() + 1..].trim();
                match arg {
                    "on"  => { self.typewriter_enabled = true;  self.typewriter_pos = 0; }
                    "off" => { self.typewriter_enabled = false; }
                    _     => { self.typewriter_enabled = !self.typewriter_enabled; self.typewriter_pos = 0; }
                }
                let state = if self.typewriter_enabled { "on" } else { "off" };
                self.entries.push(ChatEntry {
                    role: EntryRole::User,
                    content: input.clone(),
                    tool_name: String::new(),
                    time: Local::now(),
                });
                self.add_system(&format!("Typing animation: {state}"));
                return None;
            }

            if input.starts_with('/') {
                self.entries.push(ChatEntry {
                    role: EntryRole::User,
                    content: input.clone(),
                    tool_name: String::new(),
                    time: Local::now(),
                });
                return Some(Action::SlashCommand(input));
            }

            self.entries.push(ChatEntry {
                role: EntryRole::User,
                content: input.clone(),
                tool_name: String::new(),
                time: Local::now(),
            });
            self.streaming = true;
            self.active_goal = input.clone();
            self.tool_iterations = 0;
            return Some(Action::SendMessage(input));
        }

        // Forward all other keys to textarea
        self.textarea.input(key);
        self.update_suggestions();
        None
    }
}
