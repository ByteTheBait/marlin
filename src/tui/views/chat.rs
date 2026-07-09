use std::time::Instant;

use chrono::{DateTime, Local};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    layout::Size,
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget, Widget},
};
use tachyonfx::{fx, Effect, EffectRenderer, EffectTimer, Interpolation, Duration as FxDuration};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};
use tui_textarea::TextArea;

use crate::engine::{Action, UiUpdate};
use crate::tui::{
    styles::*,
    widgets::suggestions::{CmdDef, SuggestionPanel, all_commands, filter_suggestions, tab_complete},
};

// ── Chat entry ───────────────────────────────────────────────────────────────

#[derive(Clone)]
pub enum EntryRole {
    User,
    Assistant,
    System,
    Error,
    #[allow(dead_code)]
    Output,
    ToolCall,
    ToolResult { is_error: bool },
}

#[derive(Clone)]
pub struct ChatEntry {
    pub role: EntryRole,
    pub content: String,
    pub tool_name: String,
    pub time: DateTime<Local>,
}

impl ChatEntry {
    fn system(content: &str) -> Self {
        Self { role: EntryRole::System, content: content.into(), tool_name: String::new(), time: Local::now() }
    }
    fn error(content: &str) -> Self {
        Self { role: EntryRole::Error, content: content.into(), tool_name: String::new(), time: Local::now() }
    }
}

// ── Chat state ───────────────────────────────────────────────────────────────

pub struct ChatView {
    pub entries: Vec<ChatEntry>,

    // Input
    pub textarea: TextArea<'static>,
    pub input_history: Vec<String>,
    pub history_idx: i32,
    pub history_draft: String,
    pub suggestions_defs: Vec<CmdDef>,
    pub suggestions: Vec<usize>, // indices into suggestions_defs

    // Streaming
    pub streaming: bool,
    pub stream_buf: String,
    pub tool_iterations: usize,
    pub active_goal: String,
    pub current_tool: String,

    // Rate-limit
    pub rate_limited: bool,
    pub rate_limit_secs: u32,
    pub rate_limit_total: u32,

    // Approval modal
    pub approval_pending: Option<String>,

    // Scroll
    pub scroll_state: ScrollViewState,
    pub content_height: u16,
    pub viewport_height: u16,
    pub at_bottom: bool,

    pub width: u16,
    pub height: u16,
    pub provider: String,
    pub model: String,
    pub frame: u64,
    last_frame_time: Instant,
    bubble_effect: Effect,

    // Skill suggestions
    pub skills: Vec<crate::skills::SkillDef>,
    pub skill_hints: Vec<crate::tui::widgets::suggestions::SkillHint>,
    /// Skill matches sent by the engine for the last message.
    pub last_skill_matches: Vec<(String, String)>,

    // Typewriter animation
    pub typewriter_enabled: bool,
    typewriter_pos: usize,

    /// Number of built-in slash commands — used to safely append/remove user commands.
    builtin_cmd_count: usize,
}

impl ChatView {
    pub fn new(width: u16, height: u16) -> Self {
        let mut ta = TextArea::default();
        ta.set_placeholder_text("Message Marlin... (Enter to send, Ctrl+J for newline)");

        let builtin_cmds = all_commands();
        let builtin_count = builtin_cmds.len();
        Self {
            entries: vec![],
            textarea: ta,
            input_history: vec![],
            history_idx: -1,
            history_draft: String::new(),
            suggestions_defs: builtin_cmds,
            suggestions: vec![],
            streaming: false,
            stream_buf: String::new(),
            tool_iterations: 0,
            active_goal: String::new(),
            current_tool: String::new(),
            rate_limited: false,
            rate_limit_secs: 0,
            rate_limit_total: 0,
            approval_pending: None,
            scroll_state: ScrollViewState::default(),
            content_height: 0,
            viewport_height: 1,
            at_bottom: true,
            width,
            height,
            provider: String::new(),
            model: String::new(),
            frame: 0,
            last_frame_time: Instant::now(),
            bubble_effect: fx::repeating(fx::ping_pong(fx::hsl_shift_fg(
                [28.0, 0.0, 0.0],
                EffectTimer::from_ms(900, Interpolation::SineInOut),
            ))),
            skills: vec![],
            skill_hints: vec![],
            last_skill_matches: vec![],
            typewriter_enabled: false,
            typewriter_pos: 0,
            builtin_cmd_count: builtin_count,
        }
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    pub fn add_system(&mut self, text: &str) {
        self.entries.push(ChatEntry::system(text));
        self.maybe_scroll_to_bottom();
    }

    pub fn add_error(&mut self, text: &str) {
        self.entries.push(ChatEntry::error(text));
        self.maybe_scroll_to_bottom();
    }

    pub fn apply_update(&mut self, update: UiUpdate) {
        match update {
            UiUpdate::StreamChunk(chunk) => {
                if !self.streaming {
                    self.typewriter_pos = 0;
                }
                self.streaming = true;
                self.stream_buf.push_str(&chunk);
                self.maybe_scroll_to_bottom();
            }
            UiUpdate::ToolCall { name, input } => {
                self.current_tool = name.clone();
                self.entries.push(ChatEntry {
                    role: EntryRole::ToolCall,
                    content: input,
                    tool_name: name,
                    time: Local::now(),
                });
                self.maybe_scroll_to_bottom();
            }
            UiUpdate::ToolResult { name, output, is_error } => {
                self.entries.push(ChatEntry {
                    role: EntryRole::ToolResult { is_error },
                    content: output,
                    tool_name: name,
                    time: Local::now(),
                });
                self.maybe_scroll_to_bottom();
            }
            UiUpdate::SystemMsg(msg) => {
                self.add_system(&msg);
            }
            UiUpdate::ErrorMsg(msg) => {
                self.add_error(&msg);
            }
            UiUpdate::RateLimited { secs } => {
                self.rate_limited = true;
                self.rate_limit_secs = secs;
                self.rate_limit_total = secs;
                self.streaming = false;
                self.add_system(&format!("Rate limited. Resuming automatically in {secs}s..."));
            }
            UiUpdate::GoalComplete { tool_count } => {
                self.streaming = false;
                self.current_tool = String::new();
                self.typewriter_pos = 0;
                if !self.stream_buf.is_empty() {
                    let text = std::mem::take(&mut self.stream_buf);
                    self.entries.push(ChatEntry {
                        role: EntryRole::Assistant,
                        content: text,
                        tool_name: String::new(),
                        time: Local::now(),
                    });
                }
                if tool_count > 0 {
                    self.add_system(&format!("Goal complete. ({tool_count} tool calls)"));
                }
                self.active_goal.clear();
                self.tool_iterations = 0;
                self.maybe_scroll_to_bottom();
            }
            UiUpdate::StatusUpdate(info) => {
                self.provider = info.provider;
                self.model = info.model;
            }
            UiUpdate::AwaitingApproval { cmd } => {
                self.approval_pending = Some(cmd);
            }
            UiUpdate::SkillsLoaded(defs) => {
                self.skills = defs;
            }
            UiUpdate::SkillMatches(matches) => {
                self.last_skill_matches = matches;
            }
            UiUpdate::TierSelected { score, tier } => {
                self.add_system(&format!("difficulty {score}/100 → {tier} tier"));
            }
            UiUpdate::UserCommandsLoaded(defs) => {
                // Keep only built-in commands, then append loaded user commands.
                self.suggestions_defs.truncate(self.builtin_cmd_count);
                for d in defs {
                    self.suggestions_defs.push(CmdDef {
                        cmd: format!("/{}", d.name),
                        args: d.args,
                        desc: d.description,
                    });
                }
            }
            // TaskUpdate, TokenUsage, and AstMode are consumed by the runner/sidebar
            UiUpdate::TaskUpdate(_) | UiUpdate::TokenUsage { .. } | UiUpdate::AstMode(_) => {}
            UiUpdate::IndexBuilt { .. } => {}
        }
    }

    pub fn tick_rate_limit(&mut self) -> bool {
        if !self.rate_limited || self.rate_limit_secs == 0 { return false; }
        self.rate_limit_secs -= 1;
        if self.rate_limit_secs == 0 {
            self.rate_limited = false;
        }
        true
    }

    fn maybe_scroll_to_bottom(&mut self) {
        // The scroll_state is driven to the bottom inside render_viewport when at_bottom is true.
        // Calling this just keeps the at_bottom flag set; the position is applied at render time.
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

        // Ctrl+C
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if self.rate_limited || self.streaming {
                self.rate_limited = false;
                self.streaming = false;
                self.stream_buf.clear();
                self.typewriter_pos = 0;
                self.active_goal.clear();
                self.tool_iterations = 0;
                self.add_system("Cancelled.");
                return Some(Action::CancelStream);
            }
            return None;
        }

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
        if key.code == KeyCode::Up && !self.textarea.lines().iter().any(|l| l.contains('\n')) {
            if (self.history_idx + 1) < self.input_history.len() as i32 {
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
            // Fall through to viewport scroll
        }

        if key.code == KeyCode::Down && self.history_idx >= 0 {
            self.history_idx -= 1;
            let text = if self.history_idx < 0 {
                let d = std::mem::take(&mut self.history_draft);
                d
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
            self.at_bottom = false;
            self.scroll_state.scroll_up();
            self.scroll_state.scroll_up();
            self.scroll_state.scroll_up();
            return None;
        }
        if key.code == KeyCode::Down {
            self.scroll_state.scroll_down();
            self.scroll_state.scroll_down();
            self.scroll_state.scroll_down();
            let max = self.content_height.saturating_sub(self.viewport_height);
            self.at_bottom = self.scroll_state.offset().y >= max;
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
            return None;
        }
        if key.code == KeyCode::End {
            self.at_bottom = true;
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

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.frame = self.frame.wrapping_add(1);
        let tick_ms = self.last_frame_time.elapsed().as_millis().min(50) as u32;
        self.last_frame_time = Instant::now();

        // Advance typewriter position each frame while streaming
        const TYPEWRITER_SPEED: usize = 20; // chars/frame ≈ 1 200 chars/sec at 60 fps
        if self.typewriter_enabled && self.streaming && !self.stream_buf.is_empty() {
            let total = self.stream_buf.chars().count();
            self.typewriter_pos = (self.typewriter_pos + TYPEWRITER_SPEED).min(total);
        }

        let sugg_h: u16 = if !self.suggestions.is_empty() {
            (self.suggestions.len() as u16 + 2).min(10) // +2 for border
        } else if !self.skill_hints.is_empty() {
            (self.skill_hints.len() as u16 + 2).min(7)
        } else if self.streaming {
            3 // bordered bubble: top + 1 content + bottom
        } else {
            0
        };
        // Dynamic input height: 2 lines by default, expands up to 5 for multi-line (+2 for bubble border)
        let input_lines = self.textarea.lines().len().max(2);
        let input_h = (input_lines as u16).min(5) + 2;
        let sep_h = 1u16;
        let hint_h = 1u16;
        let vp_h = area.height
            .saturating_sub(sugg_h + sep_h + input_h + hint_h)
            .max(1);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(vp_h),
                Constraint::Length(sugg_h),
                Constraint::Length(sep_h),
                Constraint::Length(input_h),
                Constraint::Length(hint_h),
            ])
            .split(area);

        self.viewport_height = vp_h;
        self.render_viewport(chunks[0], buf);

        if sugg_h > 0 {
            self.render_suggestions(chunks[1], buf);
            // Pulse the bubble box with a hue-cycling effect while streaming
            if self.streaming && self.suggestions.is_empty() && self.skill_hints.is_empty() {
                buf.render_effect(
                    &mut self.bubble_effect,
                    chunks[1],
                    FxDuration::from_millis(tick_ms),
                );
            }
        }

        self.render_separator(chunks[2], buf);
        self.render_input(chunks[3], buf);
        self.render_hint(chunks[4], buf);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let style = style_separator();
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_symbol("-");
            buf[(x, area.top())].set_style(style);
        }
    }

    fn render_viewport(&mut self, area: Rect, buf: &mut Buffer) {
        self.viewport_height = area.height;

        let mut all_lines = self.build_lines(area.width as usize);

        // Live streaming buffer
        if self.streaming && !self.stream_buf.is_empty() {
            // When typewriter is on, reveal only up to typewriter_pos chars
            let tw_buf: String;
            let display_buf: &str = if self.typewriter_enabled {
                let pos = self.typewriter_pos.min(self.stream_buf.chars().count());
                tw_buf = self.stream_buf.chars().take(pos).collect();
                &tw_buf
            } else {
                &self.stream_buf
            };

            all_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("marlin", style_assistant_label()),
            ]));
            for md in render_markdown(display_buf, area.width.saturating_sub(2) as usize) {
                let mut spans = vec![Span::raw("  ")];
                spans.extend(md.spans);
                all_lines.push(Line::from(spans));
            }
            // Cursor: blinking block while caught up, underscore while still typing out
            let cursor_sym = if self.typewriter_enabled
                && self.typewriter_pos < self.stream_buf.chars().count()
            {
                "▍"
            } else {
                "|"
            };
            all_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(cursor_sym, style_prompt_active()),
            ]));
        }

        // Agentic turn indicator
        if self.streaming && self.tool_iterations > 0 && self.stream_buf.is_empty() {
            let raw = if self.current_tool.is_empty() { "thinking" } else { &self.current_tool };
            let label = if self.current_tool.is_empty() {
                "Thinking"
            } else {
                tool_display_name(&self.current_tool)
            };
            let _ = raw;
            all_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("╭", style_tool_badge_bracket()),
                Span::styled(format!(" {label} "), style_tool_badge()),
                Span::styled("╮", style_tool_badge_bracket()),
                Span::raw("  "),
                Span::styled(
                    format!("turn {}", self.tool_iterations),
                    style_system(),
                ),
            ]));
        }

        let content_height = all_lines.len() as u16;
        self.content_height = content_height;

        // Pin to bottom: manually set the y offset so the last line is always visible
        // when at_bottom is true (must be done before ScrollView clamps the offset).
        if self.at_bottom {
            let max_y = content_height.saturating_sub(area.height);
            self.scroll_state = ScrollViewState::default();
            for _ in 0..max_y {
                self.scroll_state.scroll_down();
            }
        }

        // Build the virtual scroll canvas and render lines into it
        let virtual_size = Size { width: area.width, height: content_height.max(1) };
        let mut scroll_view = ScrollView::new(virtual_size)
            .horizontal_scrollbar_visibility(ScrollbarVisibility::Never);
        scroll_view.render_widget(
            Paragraph::new(all_lines).style(style_app_bg()),
            Rect { x: 0, y: 0, width: area.width, height: content_height.max(1) },
        );

        // Render the scroll view (clips to the viewport and draws the scrollbar)
        scroll_view.render(area, buf, &mut self.scroll_state);
    }

    fn build_lines(&self, width: usize) -> Vec<Line<'static>> {
        let mut lines: Vec<Line<'static>> = Vec::new();
        let md_width = width.saturating_sub(2);
        let entries = &self.entries;
        let mut i = 0;

        while i < entries.len() {
            let entry = &entries[i];
            match &entry.role {
                EntryRole::User => {
                    let ts = entry.time.format("%H:%M").to_string();
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("you".to_string(), style_user_label()),
                        Span::styled(format!("  ·  {ts}"), style_system()),
                    ]));
                    let wrap_w = width.saturating_sub(2);
                    for l in entry.content.lines() {
                        for wrapped in word_wrap(l, wrap_w) {
                            lines.push(Line::from(Span::styled(
                                format!("  {wrapped}"),
                                style_user_text(),
                            )));
                        }
                    }
                    lines.push(Line::from(""));
                    i += 1;
                }
                EntryRole::Assistant => {
                    let ts = entry.time.format("%H:%M").to_string();
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("marlin".to_string(), style_assistant_label()),
                        Span::styled(format!("  ·  {ts}"), style_system()),
                    ]));
                    for md in render_markdown(&entry.content, md_width) {
                        let mut spans = vec![Span::raw("  ")];
                        spans.extend(md.spans);
                        lines.push(Line::from(spans));
                    }
                    lines.push(Line::from(""));
                    i += 1;
                }
                EntryRole::System => {
                    for (k, l) in entry.content.lines().enumerate() {
                        let prefix = if k == 0 { "  - " } else { "    " };
                        lines.push(Line::from(Span::styled(
                            format!("{prefix}{l}"),
                            style_system(),
                        )));
                    }
                    i += 1;
                }
                EntryRole::Error => {
                    lines.push(Line::from(Span::styled(
                        format!("  ! {}", entry.content),
                        style_error(),
                    )));
                    i += 1;
                }
                EntryRole::Output => {
                    let content_lines: Vec<&str> = entry.content.lines().collect();
                    lines.push(Line::from(Span::styled(
                        format!("  > {}", content_lines.first().copied().unwrap_or("")),
                        style_success(),
                    )));
                    for l in content_lines.iter().skip(1) {
                        lines.push(Line::from(Span::styled(
                            format!("    {l}"),
                            style_system(),
                        )));
                    }
                    i += 1;
                }
                EntryRole::ToolCall => {
                    // Collect all consecutive ToolCall entries in this batch.
                    let batch_start = i;
                    let mut batch_end = i;
                    while batch_end + 1 < entries.len()
                        && matches!(entries[batch_end + 1].role, EntryRole::ToolCall)
                    {
                        batch_end += 1;
                    }
                    let batch_size = batch_end - batch_start + 1;

                    // Collect the subsequent run of ToolResult entries (up to batch_size).
                    let result_start = batch_end + 1;
                    let mut result_count = 0;
                    while result_count < batch_size
                        && result_start + result_count < entries.len()
                        && matches!(
                            entries[result_start + result_count].role,
                            EntryRole::ToolResult { .. }
                        )
                    {
                        result_count += 1;
                    }

                    // Render each call merged with its result (if available).
                    for j in 0..batch_size {
                        let call_entry = &entries[batch_start + j];
                        let maybe_result = if j < result_count {
                            Some(&entries[result_start + j])
                        } else {
                            None
                        };

                        let display = tool_display_name(&call_entry.tool_name);
                        let mut content: Vec<(String, Style)> = Vec::new();

                        // Input args
                        let input_lines: Vec<&str> = call_entry.content.lines().collect();
                        for l in input_lines.iter().take(3) {
                            content.push((l.to_string(), style_system()));
                        }
                        if input_lines.len() > 3 {
                            content.push((
                                format!("... {} more", input_lines.len() - 3),
                                style_system(),
                            ));
                        }
                        if content.is_empty() {
                            content.push(("(empty)".to_string(), style_system()));
                        }

                        // Result (if already received)
                        if let Some(result) = maybe_result {
                            let is_err =
                                matches!(result.role, EntryRole::ToolResult { is_error: true });
                            let (icon, icon_style) =
                                if is_err { ("✗", style_error()) } else { ("✓", style_success()) };
                            content.push((icon.to_string(), icon_style));

                            let out: Vec<&str> =
                                result.content.trim_end_matches('\n').lines().collect();
                            for l in out.iter().take(6) {
                                content.push((l.to_string(), style_system()));
                            }
                            if out.len() > 6 {
                                content.push((
                                    format!("... {} more lines", out.len() - 6),
                                    style_system(),
                                ));
                            }
                        }

                        lines.extend(build_tool_bubble(display, &content, width));
                    }

                    i = result_start + result_count;
                }
                EntryRole::ToolResult { .. } => {
                    // Orphaned result with no preceding ToolCall — skip.
                    i += 1;
                }
            }
        }

        lines
    }

    fn render_suggestions(&self, area: Rect, buf: &mut Buffer) {
        let defs = &self.suggestions_defs;
        let suggs: Vec<&CmdDef> = self.suggestions.iter().map(|&i| &defs[i]).collect();
        let typed = self.textarea.lines().first().cloned().unwrap_or_default();
        let panel = SuggestionPanel {
            suggestions: &suggs,
            typed: &typed,
            width: area.width,
            frame: self.frame,
            streaming: self.streaming,
            skill_hints: &self.skill_hints,
        };
        panel.render(area, buf);
    }

    fn render_input(&self, area: Rect, buf: &mut Buffer) {
        // Draw bubble border
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(style_input_bubble());
        let inner = block.inner(area);
        block.render(area, buf);

        let lines_raw = self.textarea.lines();
        let is_empty = lines_raw.is_empty() || (lines_raw.len() == 1 && lines_raw[0].is_empty());

        // Prompt glyph — left 2 cols inside bubble
        let prompt_style = if self.streaming {
            style_system()
        } else if is_empty {
            style_prompt_empty()
        } else {
            style_prompt_active()
        };

        let area = inner;
        if area.width > 2 {
            buf[(area.x, area.y)].set_symbol(">");
            buf[(area.x, area.y)].set_style(prompt_style);
            buf[(area.x + 1, area.y)].set_symbol(" ");
        }

        let text_x = area.x + 2;
        let text_w = area.width.saturating_sub(2) as usize;

        // Placeholder when empty
        if is_empty && !self.streaming {
            let ph = "message marlin...";
            for (i, ch) in ph.chars().take(text_w).enumerate() {
                let x = text_x + i as u16;
                if x < area.right() {
                    buf[(x, area.y)].set_symbol(&ch.to_string());
                    buf[(x, area.y)].set_style(style_placeholder());
                }
            }
            return;
        }

        // Text content
        let lines_to_show: Vec<&str> = lines_raw.iter().map(String::as_str).collect();
        let show_rows = lines_to_show.len().min(area.height as usize);
        for (row, line) in lines_to_show[..show_rows].iter().enumerate() {
            let y = area.y + row as u16;
            let (x0, w) = if row == 0 {
                (text_x, text_w)
            } else {
                (area.x + 2, text_w) // keep indent on continuation rows
            };
            for (i, ch) in line.chars().take(w).enumerate() {
                let x = x0 + i as u16;
                if x < area.right() {
                    buf[(x, y)].set_symbol(&ch.to_string());
                    buf[(x, y)].set_style(style_user_text());
                }
            }
        }

        // Block cursor
        if !self.streaming {
            let last_row = show_rows.saturating_sub(1);
            let last_line = lines_to_show.get(last_row).copied().unwrap_or("");
            let col = last_line.chars().count().min(text_w);
            let cx = text_x + col as u16;
            let cy = area.y + last_row as u16;
            if cx < area.right() && cy < area.bottom() {
                buf[(cx, cy)].set_style(style_cursor());
            }
        }
    }

    fn render_hint(&self, area: Rect, buf: &mut Buffer) {
        let line = if self.rate_limited {
            let pct = if self.rate_limit_total > 0 {
                self.rate_limit_secs as f64 / self.rate_limit_total as f64
            } else { 1.0 };
            let bar = rate_bar(pct, 16);
            Line::from(vec![
                Span::styled("  rate limited  ", style_error()),
                Span::styled(bar, style_error()),
                Span::styled(
                    format!("  resuming in {}s  ctrl+c to cancel", self.rate_limit_secs),
                    style_system(),
                ),
            ])
        } else if self.streaming {
            let goal: String = self.active_goal.chars().take(40).collect();
            let ellipsis = if self.active_goal.chars().count() > 40 { "..." } else { "" };
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(format!("{goal}{ellipsis}"), style_system()),
                Span::styled(
                    format!("  ({} tool calls)  ctrl+c to cancel", self.tool_iterations),
                    Style::default().fg(col_system()),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled("  enter", style_help_key()),
                Span::styled(" send    ", style_system()),
                Span::styled("ctrl+j", style_help_key()),
                Span::styled(" newline    ", style_system()),
                Span::styled("ctrl+c", style_help_key()),
                Span::styled(" cancel    ", style_system()),
                Span::styled("/help", style_help_key()),
                Span::styled(" commands", style_system()),
            ])
        };

        Paragraph::new(line).render(area, buf);
    }
}

// ── Word wrap helper ─────────────────────────────────────────────────────────

fn word_wrap(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }
    if text.chars().count() <= max_width {
        return vec![text.to_string()];
    }
    let mut result: Vec<String> = Vec::new();
    let mut line = String::new();

    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if line.is_empty() {
            if word_len >= max_width {
                let mut chars = word.chars();
                loop {
                    let chunk: String = chars.by_ref().take(max_width).collect();
                    if chunk.is_empty() { break; }
                    if chunk.chars().count() < max_width {
                        line = chunk;
                        break;
                    }
                    result.push(chunk);
                }
            } else {
                line = word.to_string();
            }
        } else if line.chars().count() + 1 + word_len <= max_width {
            line.push(' ');
            line.push_str(word);
        } else {
            result.push(std::mem::take(&mut line));
            if word_len >= max_width {
                let mut chars = word.chars();
                loop {
                    let chunk: String = chars.by_ref().take(max_width).collect();
                    if chunk.is_empty() { break; }
                    if chunk.chars().count() < max_width {
                        line = chunk;
                        break;
                    }
                    result.push(chunk);
                }
            } else {
                line = word.to_string();
            }
        }
    }
    if !line.is_empty() {
        result.push(line);
    }
    if result.is_empty() {
        result.push(String::new());
    }
    result
}

// ── Markdown renderer (lightweight inline) ───────────────────────────────────

fn render_markdown(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;

    for raw_line in text.lines() {
        if raw_line.starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                lines.push(Line::from(Span::styled(raw_line.to_string(), style_system())));
            } else {
                lines.push(Line::from(Span::styled("```".to_string(), style_system())));
            }
            continue;
        }
        if in_code_block {
            // Wrap code lines by character (don't reformat whitespace)
            let chars: Vec<char> = raw_line.chars().collect();
            if width == 0 || chars.len() <= width {
                lines.push(Line::from(Span::styled(raw_line.to_string(), style_code_block())));
            } else {
                for chunk in chars.chunks(width.max(1)) {
                    lines.push(Line::from(Span::styled(chunk.iter().collect::<String>(), style_code_block())));
                }
            }
            continue;
        }
        if raw_line.starts_with("# ") {
            for wrapped in word_wrap(&raw_line[2..], width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(col_user()).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                )));
            }
            continue;
        }
        if raw_line.starts_with("## ") {
            for wrapped in word_wrap(&raw_line[3..], width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD),
                )));
            }
            continue;
        }
        if raw_line.starts_with("### ") {
            for wrapped in word_wrap(&raw_line[4..], width) {
                lines.push(Line::from(Span::styled(
                    wrapped,
                    Style::default().fg(col_steel()).add_modifier(Modifier::BOLD),
                )));
            }
            continue;
        }
        // Inline spans (bold, italic, code) — wrap then parse each chunk
        if width > 0 && raw_line.chars().count() > width {
            for wrapped in word_wrap(raw_line, width) {
                lines.push(parse_inline(&wrapped));
            }
        } else {
            lines.push(parse_inline(raw_line));
        }
    }

    lines
}

fn parse_inline(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut rest = line;

    while !rest.is_empty() {
        if let Some(pos) = rest.find("**") {
            spans.push(Span::styled(rest[..pos].to_string(), style_inline_text()));
            rest = &rest[pos + 2..];
            if let Some(end) = rest.find("**") {
                spans.push(Span::styled(rest[..end].to_string(), style_inline_bold()));
                rest = &rest[end + 2..];
            }
        } else if let Some(pos) = rest.find('`') {
            spans.push(Span::styled(rest[..pos].to_string(), style_inline_text()));
            rest = &rest[pos + 1..];
            if let Some(end) = rest.find('`') {
                spans.push(Span::styled(rest[..end].to_string(), style_inline_code()));
                rest = &rest[end + 1..];
            }
        } else {
            spans.push(Span::styled(rest.to_string(), style_inline_text()));
            break;
        }
    }

    if spans.is_empty() {
        Line::from(Span::styled(line.to_string(), style_inline_text()))
    } else {
        Line::from(spans)
    }
}

fn rate_bar(pct: f64, width: usize) -> String {
    let filled = (pct * width as f64) as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

fn build_tool_bubble(
    display: &str,
    content_lines: &[(String, Style)],
    width: usize,
) -> Vec<Line<'static>> {
    let mut result: Vec<Line<'static>> = Vec::new();
    let margin = 2usize;
    if width <= margin + 4 {
        return result;
    }

    let box_width = width - margin;
    let inner_width = box_width.saturating_sub(2);

    // Top border: ╭─ Name ──...──╮
    let badge_section = display.len() + 4; // "─ {name} ─"
    let fill_dashes = inner_width.saturating_sub(badge_section);
    result.push(Line::from(vec![
        Span::raw(" ".repeat(margin)),
        Span::styled("╭─ ".to_string(), style_tool_badge_bracket()),
        Span::styled(display.to_string(), style_tool_badge()),
        Span::styled(" ─".to_string(), style_tool_badge_bracket()),
        Span::styled("─".repeat(fill_dashes), style_tool_badge_bracket()),
        Span::styled("╮".to_string(), style_tool_badge_bracket()),
    ]));

    // Content lines: │ text padded │
    let text_area = inner_width.saturating_sub(1);
    for (text, style) in content_lines {
        let chars: String = text.chars().take(text_area).collect();
        let pad_len = text_area.saturating_sub(chars.chars().count());
        result.push(Line::from(vec![
            Span::raw(" ".repeat(margin)),
            Span::styled("│".to_string(), style_tool_badge_bracket()),
            Span::raw(" "),
            Span::styled(chars, *style),
            Span::raw(" ".repeat(pad_len)),
            Span::styled("│".to_string(), style_tool_badge_bracket()),
        ]));
    }

    // Bottom border: ╰──...──╯
    result.push(Line::from(vec![
        Span::raw(" ".repeat(margin)),
        Span::styled("╰".to_string(), style_tool_badge_bracket()),
        Span::styled("─".repeat(inner_width), style_tool_badge_bracket()),
        Span::styled("╯".to_string(), style_tool_badge_bracket()),
    ]));

    result
}

fn tool_display_name(raw: &str) -> &'static str {
    match raw {
        "read_file"        => "Read",
        "write_file"       => "Write",
        "edit_file"        => "Update",
        "run_command"      => "Run",
        "list_directory"   => "List",
        "create_directory" => "Mkdir",
        "search_codebase"  => "Search",
        "run_skill"        => "Skill",
        "ast_skeleton"     => "Skeleton",
        "ast_get_node"     => "Node",
        "ast_mutate"       => "Mutate",
        _                  => "Tool",
    }
}
