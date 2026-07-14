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
    label: &'static str,
    kind: FieldKind,
    options: Vec<String>,
    current: String,
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
}

const MAX_TOKEN_PRESETS: [usize; 6] = [1024, 2048, 4096, 8192, 16384, 32768];

impl ConfigMenu {
    pub fn new(state: ConfigState, animate: bool) -> Self {
        Self { state, selected: 0, animate, editing: false, edit_buf: String::new() }
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
        vec![
            MenuItem {
                key: "provider",
                label: "Provider",
                kind: FieldKind::Cycle,
                options: s.providers.clone(),
                current: s.provider.clone(),
            },
            MenuItem {
                key: "api_key",
                label: "API key",
                kind: FieldKind::Text,
                options: vec![],
                current: Self::mask_key(&s.api_key),
            },
            MenuItem {
                key: "model",
                label: "Model",
                kind: FieldKind::Cycle,
                options: models,
                current: s.model.clone(),
            },
            MenuItem {
                key: "theme",
                label: "Theme",
                kind: FieldKind::Cycle,
                options: vec!["dark".into(), "light".into()],
                current: s.theme.clone(),
            },
            MenuItem {
                key: "sandbox",
                label: "Sandbox",
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "permissive".into(), "mxc".into()],
                current: s.sandbox_mode.clone(),
            },
            MenuItem {
                key: "permissions",
                label: "Permissions",
                kind: FieldKind::Cycle,
                options: vec!["require".into(), "skip".into()],
                current: if s.skip_permissions { "skip" } else { "require" }.into(),
            },
            MenuItem {
                key: "clean_env",
                label: "Clean env",
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "on".into()],
                current: onoff(s.clean_env),
            },
            MenuItem {
                key: "ast",
                label: "AST mode",
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "sexpr".into(), "harness".into()],
                current: s.ast_mode.clone(),
            },
            MenuItem {
                key: "subagents",
                label: "Skill subagents",
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "on".into()],
                current: onoff(s.skill_subagents),
            },
            MenuItem {
                key: "animate",
                label: "Animate",
                kind: FieldKind::Cycle,
                options: vec!["off".into(), "on".into()],
                current: onoff(self.animate),
            },
            MenuItem {
                key: "max_tokens",
                label: "Max tokens",
                kind: FieldKind::Cycle,
                options: tokens.iter().map(|t| t.to_string()).collect(),
                current: s.max_tokens.to_string(),
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

    /// Right/Enter/Space on the selected row: cycles fixed-option rows, or
    /// opens inline text edit for free-text rows (e.g. API key).
    fn activate(&mut self) -> ConfigMenuOutcome {
        let items = self.items();
        let Some(item) = items.get(self.selected) else {
            return ConfigMenuOutcome::None;
        };
        if item.kind == FieldKind::Text {
            self.editing = true;
            self.edit_buf.clear();
            return ConfigMenuOutcome::None;
        }
        self.cycle(1)
    }

    fn on_key_editing(&mut self, key: KeyEvent) -> ConfigMenuOutcome {
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
