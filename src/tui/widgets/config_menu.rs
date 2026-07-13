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

struct MenuItem {
    key: &'static str,
    label: &'static str,
    options: Vec<String>,
    current: String,
}

/// Interactive settings overlay opened with /config. Values are cycled with
/// ←/→ and applied immediately; the engine echoes a refreshed snapshot after
/// each change so dependent rows (e.g. the model list) stay in sync.
pub struct ConfigMenu {
    pub state: ConfigState,
    selected: usize,
}

const MAX_TOKEN_PRESETS: [usize; 6] = [1024, 2048, 4096, 8192, 16384, 32768];

impl ConfigMenu {
    pub fn new(state: ConfigState) -> Self {
        Self { state, selected: 0 }
    }

    /// Replace the snapshot (engine refresh) without losing cursor position.
    pub fn sync(&mut self, state: ConfigState) {
        self.state = state;
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
                options: s.providers.clone(),
                current: s.provider.clone(),
            },
            MenuItem {
                key: "model",
                label: "Model",
                options: models,
                current: s.model.clone(),
            },
            MenuItem {
                key: "theme",
                label: "Theme",
                options: vec!["dark".into(), "light".into()],
                current: s.theme.clone(),
            },
            MenuItem {
                key: "sandbox",
                label: "Sandbox",
                options: vec!["off".into(), "permissive".into(), "mxc".into()],
                current: s.sandbox_mode.clone(),
            },
            MenuItem {
                key: "permissions",
                label: "Permissions",
                options: vec!["require".into(), "skip".into()],
                current: if s.skip_permissions { "skip" } else { "require" }.into(),
            },
            MenuItem {
                key: "clean_env",
                label: "Clean env",
                options: vec!["off".into(), "on".into()],
                current: onoff(s.clean_env),
            },
            MenuItem {
                key: "ast",
                label: "AST mode",
                options: vec!["off".into(), "sexpr".into(), "harness".into()],
                current: s.ast_mode.clone(),
            },
            MenuItem {
                key: "subagents",
                label: "Skill subagents",
                options: vec!["off".into(), "on".into()],
                current: onoff(s.skill_subagents),
            },
            MenuItem {
                key: "max_tokens",
                label: "Max tokens",
                options: tokens.iter().map(|t| t.to_string()).collect(),
                current: s.max_tokens.to_string(),
            },
        ]
    }

    pub fn on_key(&mut self, key: KeyEvent) -> ConfigMenuOutcome {
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
                self.cycle(1)
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
    fn apply_local(&mut self, key: &str, value: &str) {
        let s = &mut self.state;
        match key {
            "provider" => s.provider = value.into(),
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
            let mut value: String = item.current.clone();
            if value.chars().count() > value_w {
                value = value.chars().take(value_w.saturating_sub(1)).collect::<String>() + "…";
            }
            let (label_style, arrow_style, value_style) = if selected {
                (
                    Style::default().fg(col_user()).add_modifier(Modifier::BOLD),
                    Style::default().fg(col_aqua()),
                    Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD),
                )
            } else {
                (style_system(), style_app_bg(), Style::default().fg(col_steel()))
            };
            let (l_arrow, r_arrow) = if selected { ("◂ ", " ▸") } else { ("  ", "  ") };
            lines.push(Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{:<label_w$}", item.label), label_style),
                Span::styled(l_arrow, arrow_style),
                Span::styled(value, value_style),
                Span::styled(r_arrow, arrow_style),
            ]));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(" ↑↓", style_help_key()),
            Span::styled(" move   ", style_system()),
            Span::styled("←→", style_help_key()),
            Span::styled(" change   ", style_system()),
            Span::styled("esc", style_help_key()),
            Span::styled(" close", style_system()),
        ]));

        Paragraph::new(lines).style(style_app_bg()).render(inner, buf);
    }
}
