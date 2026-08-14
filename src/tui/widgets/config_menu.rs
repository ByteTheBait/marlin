use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
};

use crate::engine::ConfigState;
use crate::tui::styles::*;

/// Result of feeding a key event to the config menu.
pub enum ConfigMenuOutcome {
    None,
    /// User closed the menu (Esc / q).
    Close,
    /// A setting was cycled to a new value — forward to the engine.
    Set { key: &'static str, value: String },
}

/// Whether a row cycles through fixed options with ←/→ or is edited as free
/// text (e.g. the API key, which can't be enumerated).
#[derive(PartialEq)]
enum FieldKind {
    Cycle,
    Text,
}

struct MenuItem {
    key: &'static str,
    label: String,
    kind: FieldKind,
    options: Vec<String>,
    current: String,
}

/// Step within the "New provider" wizard — gathers name, URL, model, and API
/// key one at a time (each a separate Enter-to-confirm text entry on the same
/// row) before creating the provider in one shot.
#[derive(Clone, Copy, PartialEq)]
enum NewProviderField {
    Name,
    Endpoint,
    Model,
    ApiKey,
}

impl NewProviderField {
    fn label(&self) -> &'static str {
        match self {
            NewProviderField::Name => "New: name",
            NewProviderField::Endpoint => "New: URL",
            NewProviderField::Model => "New: model",
            NewProviderField::ApiKey => "New: key",
        }
    }
}

struct NewProviderWizard {
    field: NewProviderField,
    name: String,
    endpoint: String,
    model: String,
}

/// Interactive settings overlay opened with /config. Cycle rows are changed
/// with ←/→ and applied immediately; the engine echoes a refreshed snapshot
/// after each change so dependent rows (e.g. the model list, or the API key
/// field when the provider changes) stay in sync. Text rows (API key) enter
/// an inline edit mode on Enter.
pub struct ConfigMenu {
    pub state: ConfigState,
    selected: usize,
    /// Typewriter animation toggle — lives in the TUI layer (ChatView), not
    /// the engine's ConfigState, so it's tracked locally and intercepted by
    /// the caller instead of round-tripping through Action::ConfigSet.
    animate: bool,
    editing: bool,
    edit_buf: String,
    /// Set while stepping through the "New provider" row's name/URL/model/key
    /// sequence; `None` for ordinary single-value text rows (e.g. API key).
    new_provider: Option<NewProviderWizard>,
}

const MAX_TOKEN_PRESETS: [usize; 10] =
    [1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144, 524288];

impl ConfigMenu {
    pub fn new(state: ConfigState, animate: bool) -> Self {
        Self { state, selected: 0, animate, editing: false, edit_buf: String::new(), new_provider: None }
    }

    /// Replace the snapshot (engine refresh) without losing cursor position.
    pub fn sync(&mut self, state: ConfigState) {
        self.state = state;
    }

    /// Masked display for a secret value: keeps the last 4 chars visible.
    fn mask_key(key: &str) -> String {
        if key.is_empty() {
            return "(not set)".into();
        }
        let n = key.chars().count();
        if n <= 4 {
            return "•".repeat(n);
        }
        let tail: String = key.chars().skip(n - 4).collect();
        format!("{}{}", "•".repeat((n - 4).min(16)), tail)
    }

    fn items(&self) -> Vec<MenuItem> {
        let s = &self.state;
        let mut models = s.models.clone();
        if !models.contains(&s.model) {
            models.insert(0, s.model.clone());
        }
        let mut tokens: Vec<usize> = MAX_TOKEN_PRESETS.to_vec();
        if !tokens.contains(&s.max_tokens) {
            tokens.push(s.max_tokens);
            tokens.sort_unstable();
        }
        let onoff = |b: bool| if b { "on" } else { "off" }.to_string();
        let (np_label, np_current) = match &self.new_provider {
            Some(w) => (w.field.label().to_string(), String::new()),
            None => ("New provider".to_string(), "(name, enter)".to_string()),
        };
        vec![
            MenuItem {
                key: "provider",
                label: "Provider".into(),
                kind: FieldKind::Cycle,
                options: s.providers.clone(),
                current: s.provider.clone(),
            },
            MenuItem {
                key: "new_provider",
                label: np_label,
                kind: FieldKind::Text,
                options: vec![],
                current: np_current,
            },
            MenuItem {
                key: "api_key",
                label: "API key".into(),
                kind: FieldKind::Text,
                options: vec![],
                current: Self::mask_key(&s.api_key),
            },
            MenuItem {
                key: "model",
                label: "Model".into(),
                kind: FieldKind::Cycle,
                options: models,
                current: s.model.clone(),
            },
            MenuItem {
                key: "theme",
                label: "Theme".into(),
                kind: FieldKind::Cycle,
                options: {
                    let mut opts = vec!["dark".into(), "light".into()];
                    opts.extend(s.named_themes.iter().cloned());
                    opts
                },
                current: s.theme.clone(),
            },
            MenuItem {
                key: "sandbox",
                label: "Sandbox".into(),
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "permissive".into(), "mxc".into()],
                current: s.sandbox_mode.clone(),
            },
            MenuItem {
                key: "permissions",
                label: "Permissions".into(),
                kind: FieldKind::Cycle,
                options: vec!["require".into(), "skip".into()],
                current: if s.skip_permissions { "skip" } else { "require" }.into(),
            },
            MenuItem {
                key: "clean_env",
                label: "Clean env".into(),
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "on".into()],
                current: onoff(s.clean_env),
            },
            MenuItem {
                key: "ast",
                label: "AST mode".into(),
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "sexpr".into(), "harness".into()],
                current: s.ast_mode.clone(),
            },
            MenuItem {
                key: "subagents",
                label: "Skill subagents".into(),
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "on".into()],
                current: onoff(s.skill_subagents),
            },
            MenuItem {
                key: "animate",
                label: "Animate".into(),
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "on".into()],
                current: onoff(self.animate),
            },
            MenuItem {
                key: "max_tokens",
                label: "Max tokens".into(),
                kind: FieldKind::Cycle,
                options: tokens.iter().map(|t| t.to_string()).collect(),
                current: s.max_tokens.to_string(),
            },
            MenuItem {
                key: "tool_call_limit",
                label: "Tool call limit".into(),
                kind: FieldKind::Text,
                options: vec![],
                current: s.tool_call_limit.to_string(),
            },
        ]
    }

    pub fn on_key(&mut self, key: KeyEvent) -> ConfigMenuOutcome {
        if self.editing {
            return self.on_key_editing(key);
        }
        let count = self.items().len();
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => ConfigMenuOutcome::Close,
            KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab => {
                self.selected = self.selected.checked_sub(1).unwrap_or(count - 1);
                ConfigMenuOutcome::None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1) % count;
                ConfigMenuOutcome::None
            }
            KeyCode::Left | KeyCode::Char('h') => self.cycle(-1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char(' ') => {
                self.activate()
            }
            _ => ConfigMenuOutcome::None,
        }
    }

    /// Right/Enter/Space on the selected row: cycles fixed-option rows, opens
    /// the name/URL/model/key wizard for "New provider", or opens inline text
    /// edit for other free-text rows (e.g. API key).
    fn activate(&mut self) -> ConfigMenuOutcome {
        let items = self.items();
        let Some(item) = items.get(self.selected) else {
            return ConfigMenuOutcome::None;
        };
        if item.key == "new_provider" {
            self.new_provider = Some(NewProviderWizard {
                field: NewProviderField::Name,
                name: String::new(),
                endpoint: String::new(),
                model: String::new(),
            });
            self.editing = true;
            self.edit_buf.clear();
            return ConfigMenuOutcome::None;
        }
        if item.kind == FieldKind::Text {
            self.editing = true;
            self.edit_buf.clear();
            return ConfigMenuOutcome::None;
        }
        self.cycle(1)
    }

    fn on_key_editing(&mut self, key: KeyEvent) -> ConfigMenuOutcome {
        if self.new_provider.is_some() {
            return self.on_key_new_provider(key);
        }
        match key.code {
            KeyCode::Esc => {
                self.editing = false;
                self.edit_buf.clear();
                ConfigMenuOutcome::None
            }
            KeyCode::Enter => {
                self.editing = false;
                let items = self.items();
                let Some(item) = items.get(self.selected) else {
                    return ConfigMenuOutcome::None;
                };
                let key = item.key;
                let value = std::mem::take(&mut self.edit_buf);
                self.apply_local(key, &value);
                ConfigMenuOutcome::Set { key, value }
            }
            KeyCode::Backspace => {
                self.edit_buf.pop();
                ConfigMenuOutcome::None
            }
            KeyCode::Char(c) => {
                self.edit_buf.push(c);
                ConfigMenuOutcome::None
            }
            _ => ConfigMenuOutcome::None,
        }
    }

    /// Steps the "New provider" row through name → URL → model → key, one
    /// Enter-confirmed text entry per field, then emits a single `Set` whose
    /// value packs all four as newline-separated fields (Enter is the only
    /// way to commit a field, so a literal newline can never end up in
    /// `edit_buf`). `apply_config_set` on the engine side splits it back out.
    fn on_key_new_provider(&mut self, key: KeyEvent) -> ConfigMenuOutcome {
        match key.code {
            KeyCode::Esc => {
                self.new_provider = None;
                self.editing = false;
                self.edit_buf.clear();
                ConfigMenuOutcome::None
            }
            KeyCode::Enter => {
                let value = std::mem::take(&mut self.edit_buf).trim().to_string();
                let wiz = self.new_provider.as_mut().expect("checked by caller");
                match wiz.field {
                    NewProviderField::Name => {
                        if value.is_empty() {
                            return ConfigMenuOutcome::None;
                        }
                        wiz.name = value;
                        wiz.field = NewProviderField::Endpoint;
                        ConfigMenuOutcome::None
                    }
                    NewProviderField::Endpoint => {
                        wiz.endpoint = value;
                        wiz.field = NewProviderField::Model;
                        ConfigMenuOutcome::None
                    }
                    NewProviderField::Model => {
                        wiz.model = value;
                        wiz.field = NewProviderField::ApiKey;
                        ConfigMenuOutcome::None
                    }
                    NewProviderField::ApiKey => {
                        let encoded = format!("{}\n{}\n{}\n{}", wiz.name, wiz.endpoint, wiz.model, value);
                        self.new_provider = None;
                        self.editing = false;
                        ConfigMenuOutcome::Set { key: "new_provider", value: encoded }
                    }
                }
            }
            KeyCode::Backspace => {
                self.edit_buf.pop();
                ConfigMenuOutcome::None
            }
            KeyCode::Char(c) => {
                self.edit_buf.push(c);
                ConfigMenuOutcome::None
            }
            _ => ConfigMenuOutcome::None,
        }
    }

    fn cycle(&mut self, dir: i32) -> ConfigMenuOutcome {
        let items = self.items();
        let Some(item) = items.get(self.selected) else {
            return ConfigMenuOutcome::None;
        };
        let n = item.options.len() as i32;
        if n < 2 {
            return ConfigMenuOutcome::None;
        }
        let idx = item.options.iter().position(|o| *o == item.current).unwrap_or(0) as i32;
        let value = item.options[((idx + dir + n) % n) as usize].clone();
        if value == item.current {
            return ConfigMenuOutcome::None;
        }
        let key = item.key;
        self.apply_local(key, &value);
        ConfigMenuOutcome::Set { key, value }
    }

    /// Insert pasted text into the active text field. Only meaningful while
    /// editing (API key row or the new-provider wizard); otherwise the paste
    /// is ignored so it can't leak into the chat input behind the menu.
    pub fn paste(&mut self, text: &str) {
        if self.editing {
            self.edit_buf.push_str(text);
        }
    }

    /// Optimistic local update so the row changes instantly; the engine's
    /// ConfigState echo is authoritative and overwrites this shortly after.
    /// `animate` is the one exception — it's TUI-local and never echoed back.
    fn apply_local(&mut self, key: &str, value: &str) {
        if key == "animate" {
            self.animate = value == "on";
            return;
        }
        let s = &mut self.state;
        match key {
            "provider" => s.provider = value.into(),
            "api_key" => s.api_key = value.into(),
            "model" => s.model = value.into(),
            "theme" => s.theme = value.into(),
            "sandbox" => s.sandbox_mode = value.into(),
            "permissions" => s.skip_permissions = value == "skip",
            "clean_env" => s.clean_env = value == "on",
            "ast" => s.ast_mode = value.into(),
            "subagents" => s.skill_subagents = value == "on",
            "max_tokens" => s.max_tokens = value.parse().unwrap_or(s.max_tokens),
            "tool_call_limit" => s.tool_call_limit = value.parse().unwrap_or(s.tool_call_limit),
            _ => {}
        }
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let items = self.items();
        let modal_w = area.width.clamp(44, 64);
        let modal_h = (items.len() as u16 + 5).min(area.height);
        let x = area.x + (area.width.saturating_sub(modal_w)) / 2;
        let y = area.y + (area.height.saturating_sub(modal_h)) / 2;
        let modal = Rect { x, y, width: modal_w, height: modal_h };

        Clear.render(modal, buf);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(col_cobalt()))
            .title(Span::styled(
                " Settings ",
                Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD),
            ));
        let inner = block.inner(modal);
        block.render(modal, buf);

        let label_w = 16usize;
        let value_w = (inner.width as usize).saturating_sub(label_w + 7);
        let mut lines: Vec<Line> = vec![Line::from("")];
        for (i, item) in items.iter().enumerate() {
            let selected = i == self.selected;
            let editing_this = selected && self.editing;
            let mut value: String = if editing_this {
                format!("{}▏", self.edit_buf)
            } else {
                item.current.clone()
            };
            if value.chars().count() > value_w {
                let take = if editing_this { value.chars().count() - value_w } else { 0 };
                value = if editing_this {
                    // While typing, keep the tail (and cursor) in view.
                    value.chars().skip(take).collect()
                } else {
                    value.chars().take(value_w.saturating_sub(1)).collect::<String>() + "…"
                };
            }
            let (label_style, arrow_style, value_style) = if editing_this {
                (
                    Style::default().fg(col_user()).add_modifier(Modifier::BOLD),
                    Style::default().fg(col_aqua()),
                    Style::default().fg(col_success()).add_modifier(Modifier::BOLD),
                )
            } else if selected {
                (
                    Style::default().fg(col_user()).add_modifier(Modifier::BOLD),
                    Style::default().fg(col_aqua()),
                    Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD),
                )
            } else {
                (style_system(), style_app_bg(), Style::default().fg(col_steel()))
            };
            let (l_arrow, r_arrow) = if editing_this {
                ("> ", "  ")
            } else if selected {
                ("◂ ", " ▸")
            } else {
                ("  ", "  ")
            };
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{:<label_w$}", item.label), label_style),
                Span::styled(l_arrow, arrow_style),
                Span::styled(value, value_style),
                Span::styled(r_arrow, arrow_style),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(if self.editing {
            Line::from(vec![
                Span::styled(" type", style_help_key()),
                Span::styled(" edit   ", style_system()),
                Span::styled("enter", style_help_key()),
                Span::styled(" confirm   ", style_system()),
                Span::styled("esc", style_help_key()),
                Span::styled(" cancel", style_system()),
            ])
        } else {
            Line::from(vec![
                Span::styled(" ↑↓", style_help_key()),
                Span::styled(" move   ", style_system()),
                Span::styled("←→", style_help_key()),
                Span::styled(" change   ", style_system()),
                Span::styled("esc", style_help_key()),
                Span::styled(" close", style_system()),
            ])
        });

        Paragraph::new(lines).style(style_app_bg()).render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ConfigState;

    fn menu() -> ConfigMenu {
        ConfigMenu::new(ConfigState::default(), false)
    }

    #[test]
    fn paste_ignored_when_not_editing() {
        let mut m = menu();
        m.paste("secret-key");
        assert!(m.edit_buf.is_empty());
    }

    #[test]
    fn paste_appends_to_edit_buf_while_editing() {
        let mut m = menu();
        // Open inline edit on the API key row (index 2).
        m.selected = 2;
        m.activate();
        assert!(m.editing);
        m.paste("sk-abc");
        m.paste("def");
        assert_eq!(m.edit_buf, "sk-abcdef");
    }

    #[test]
    fn paste_into_new_provider_wizard() {
        let mut m = menu();
        // "New provider" row is index 1.
        m.selected = 1;
        m.activate();
        assert!(m.editing);
        assert!(m.new_provider.is_some());
        m.paste("my-provider");
        assert_eq!(m.edit_buf, "my-provider");
    }

    #[test]
    fn tool_call_limit_row_is_editable_text() {
        let mut m = menu();
        // Find the "Tool call limit" row.
        let idx = m.items().iter().position(|i| i.key == "tool_call_limit").unwrap();
        m.selected = idx;
        m.activate();
        assert!(m.editing);
        m.paste("250");
        assert_eq!(m.edit_buf, "250");
        // Enter commits it as a Set.
        let outcome = m.on_key(KeyEvent::from(KeyCode::Enter));
        assert!(matches!(outcome, ConfigMenuOutcome::Set { key: "tool_call_limit", value } if value == "250"));
        assert_eq!(m.state.tool_call_limit, 250);
    }
}
