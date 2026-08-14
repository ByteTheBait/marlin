use std::time::Instant;

use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect, Size},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, StatefulWidget, Widget},
};
use tachyonfx::{Duration as FxDuration, EffectRenderer};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::tui::styles::*;
use crate::tui::widgets::suggestions::{CmdDef, SuggestionPanel};

use super::entry::EntryRole;
use super::helpers::{build_tool_bubble, rate_bar, tool_display_name};
use super::markdown::{render_markdown, word_wrap};
use super::state::ChatView;

impl ChatView {
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
        let session_h = 1u16;
        let vp_h = area.height
            .saturating_sub(sugg_h + sep_h + input_h + hint_h + session_h)
            .max(1);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(vp_h),
                Constraint::Length(sugg_h),
                Constraint::Length(sep_h),
                Constraint::Length(session_h),
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
        self.render_session_status(chunks[3], buf);
        self.render_input(chunks[4], buf);
        self.render_hint(chunks[5], buf);
    }

    fn render_separator(&self, area: Rect, buf: &mut Buffer) {
        let style = style_separator();
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_symbol("-");
            buf[(x, area.top())].set_style(style);
        }
    }

    /// Session status bar shown directly above the input box: working directory
    /// and the current goal/summary. Its background color is configurable per
    /// working directory via `/color <#rrggbb>` so different sessions are easy
    /// to tell apart at a glance.
    fn render_session_status(&self, area: Rect, buf: &mut Buffer) {
        let bg = self.status_color
            .map(|[r, g, b]| Color::Rgb(r, g, b))
            .unwrap_or_else(style_session_status_bg);
        let fg = style_session_status_fg();

        // Working directory (basename, or full path if it's short).
        let dir = if self.work_dir.is_empty() {
            ".".to_string()
        } else {
            let base = std::path::Path::new(&self.work_dir)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| self.work_dir.clone());
            if base.is_empty() { self.work_dir.clone() } else { base }
        };

        // Goal / summary of the current session.
        let goal = if self.active_goal.is_empty() {
            "no active goal".to_string()
        } else {
            self.active_goal.clone()
        };

        let line = Line::from(vec![
            Span::styled("  ", Style::default().bg(bg)),
            Span::styled("📁", Style::default().bg(bg).fg(fg)),
            Span::styled(format!(" {dir} "), Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD)),
            Span::styled("·", Style::default().bg(bg).fg(fg)),
            Span::styled(format!(" {goal}"), Style::default().bg(bg).fg(fg)),
        ]);

        // Fill the whole row with the background color, then draw the text.
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_style(Style::default().bg(bg));
        }
        Paragraph::new(line).render(area, buf);
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

            let stream_start = all_lines.len();
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
            // Pulse the cursor color while streaming for a subtle "alive" feel.
            let cursor_style = style_cursor_pulse(self.frame);
            all_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(cursor_sym, cursor_style),
            ]));
            // Apply a left-border accent to the live streaming buffer so it reads
            // as "in progress" rather than a committed entry.
            let hl = style_stream_highlight();
            for line in &mut all_lines[stream_start..] {
                line.spans.insert(0, Span::styled("▎", hl));
            }
        }
        if self.streaming && self.tool_iterations > 0 && self.stream_buf.is_empty() {
            let raw = if self.current_tool.is_empty() { "thinking" } else { &self.current_tool };
            let label = if self.current_tool.is_empty() {
                "Thinking"
            } else {
                tool_display_name(&self.current_tool)
            };
            let _ = raw;
            // Pulse the tool badge while the tool is actively running.
            let badge_style = if self.streaming {
                style_tool_badge_pulse(self.frame)
            } else {
                style_tool_badge()
            };
            all_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled("╭", style_tool_badge_bracket()),
                Span::styled(format!(" {} {label} ", tool_glyph(&self.current_tool)), badge_style),
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

        // Pin to bottom: when at_bottom is true, ease the viewport toward the
        // bottom instead of jumping. The step is a fraction of the remaining
        // distance, so it glides fast when there's a lot to scroll through and
        // slows down as it approaches (e.g. when the model response stops).
        //
        // The smooth easing is tied to the typewriter animation toggle
        // (`/animate`): when animation is off, the viewport snaps straight to
        // the bottom instead of gliding, so the two "animated" behaviors can
        // be turned off together.
        if self.at_bottom {
            let max_y = content_height.saturating_sub(area.height) as f64;
            // Snap if animation is off, the target was explicitly requested
            // (End / send), or the content now fits entirely in the viewport.
            if !self.typewriter_enabled || self.smooth_offset >= f64::MAX / 2.0 || max_y <= 0.0 {
                self.smooth_offset = max_y;
            } else {
                let remaining = max_y - self.smooth_offset;
                if remaining > 0.5 {
                    // Move a fraction of the remaining distance each frame —
                    // larger absolute step when far, smaller as we near the
                    // bottom. A floor keeps it from stalling on tiny gaps.
                    let step = (remaining * 0.18).max(0.5).min(remaining);
                    self.smooth_offset += step;
                } else {
                    self.smooth_offset = max_y;
                }
            }
            self.smooth_offset = self.smooth_offset.min(max_y);

            self.scroll_state = ScrollViewState::default();
            for _ in 0..self.smooth_offset.round() as u16 {
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

        // Scroll-to-bottom hint: when new content arrives while the user has
        // scrolled up, show a small "▼ new" indicator at the bottom of the
        // viewport so they know there's unread content below.
        if self.new_content_arrived && !self.at_bottom && area.height >= 2 {
            let label = "  ▼ new  ";
            let x = area.right().saturating_sub(label.len() as u16).saturating_sub(1);
            let y = area.bottom().saturating_sub(2);
            // Pulse the hint so it gently draws the eye without being obtrusive.
            let hint_style = style_scroll_hint_pulse(self.frame);
            for (i, ch) in label.chars().enumerate() {
                let cell_x = x + i as u16;
                if cell_x < area.right() && y < area.bottom() {
                    buf[(cell_x, y)].set_symbol(&ch.to_string());
                    buf[(cell_x, y)].set_style(hint_style);
                }
            }
        }
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
                EntryRole::Summary => {
                    // Muted closing note from mark_complete — grayed-out text,
                    // no label, no tool bubble.
                    for md in render_markdown(&entry.content, md_width) {
                        let mut spans = vec![Span::raw("  ")];
                        for s in md.spans {
                            spans.push(Span::styled(s.content, style_summary()));
                        }
                        lines.push(Line::from(spans));
                    }
                    lines.push(Line::from(""));
                    i += 1;
                }
                EntryRole::Error => {
                    lines.push(Line::from(Span::styled(
                        format!("  ! {}", entry.content),
                        style_error(),
                    )));
                    i += 1;
                }
                EntryRole::Steer => {
                    // A steering note/command result — rendered as a distinct
                    // text field in the model's output area. Amber "steer" label
                    // so it reads as user input injected mid-stream, not model
                    // output.
                    lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("steer".to_string(), style_steer_label()),
                    ]));
                    for md in render_markdown(&entry.content, md_width) {
                        let mut spans = vec![Span::raw("  ")];
                        for s in md.spans {
                            spans.push(Span::styled(s.content, style_steer_text()));
                        }
                        lines.push(Line::from(spans));
                    }
                    lines.push(Line::from(""));
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
                        let display = format!("{} {display}", tool_glyph(&call_entry.tool_name));
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
                        } else if !self.tool_stream_buf.is_empty() {
                            // No result yet — show the live streaming output from
                            // the long-running tool inside the bubble, so it stays
                            // in the tool box rather than bleeding into the chat.
                            let out: Vec<&str> =
                                self.tool_stream_buf.trim_end_matches('\n').lines().collect();
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

                        lines.extend(build_tool_bubble(&display, &content, width));
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
        // Draw bubble border — pulse the border color while streaming so the
        // input area reads as "busy" without being distracting.
        let border_style = if self.streaming {
            style_input_bubble_pulse(self.frame)
        } else {
            style_input_bubble()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(border_style);
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
