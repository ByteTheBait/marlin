use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Paragraph, Widget},
};

use crate::engine::tasks::{TaskStatus, TaskStep};
use crate::tui::styles::*;

const TOKEN_HISTORY_LEN: usize = 60;
/// Bound on the sidebar's subagent list — running entries are always kept
/// (there's normally only one at a time); this caps how many finished ones
/// linger below them over a long session.
const MAX_SUBAGENT_ENTRIES: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SubagentStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone)]
pub struct SubagentEntry {
    pub id: String,
    pub label: String,
    pub status: SubagentStatus,
    pub current_tool: Option<String>,
}

pub struct Sidebar {
    pub token_used: usize,
    pub token_budget: usize,
    pub token_history: Vec<usize>,
    pub tasks: Vec<TaskStep>,
    /// Upfront plan (see `Engine::maybe_generate_plan`) — rendered above `tasks`
    /// when non-empty; a coarse ordered checklist, not a replacement for it.
    pub plan: Vec<TaskStep>,
    pub subagents: Vec<SubagentEntry>,
}

impl Sidebar {
    pub fn new() -> Self {
        Self {
            token_used: 0,
            token_budget: 100_000,
            token_history: Vec::new(),
            tasks: vec![],
            plan: vec![],
            subagents: vec![],
        }
    }

    pub fn push_token_sample(&mut self, used: usize) {
        self.token_history.push(used);
        if self.token_history.len() > TOKEN_HISTORY_LEN {
            self.token_history.remove(0);
        }
    }

    pub fn subagent_started(&mut self, id: String, label: String) {
        // Trim finished entries (never running ones) before adding a new one.
        while self.subagents.len() >= MAX_SUBAGENT_ENTRIES {
            let Some(idx) = self.subagents.iter().position(|s| s.status != SubagentStatus::Running) else { break };
            self.subagents.remove(idx);
        }
        self.subagents.push(SubagentEntry { id, label, status: SubagentStatus::Running, current_tool: None });
    }

    pub fn subagent_tool_call(&mut self, id: &str, name: String) {
        if let Some(s) = self.subagents.iter_mut().find(|s| s.id == id) {
            s.current_tool = Some(name);
        }
    }

    pub fn subagent_finished(&mut self, id: &str, ok: bool) {
        if let Some(s) = self.subagents.iter_mut().find(|s| s.id == id) {
            s.status = if ok { SubagentStatus::Done } else { SubagentStatus::Failed };
            s.current_tool = None;
        }
    }
}

impl Widget for Sidebar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 4 { return; }

        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(style_separator());
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.height == 0 { return; }

        let mut y = inner.y;

        // ── Token budget section ───────────────────────────────────────────
        let header_style = Style::default()
            .fg(fg_on_app_bg())
            .add_modifier(Modifier::BOLD);
        let _ = header_style; // silence unused warning - recompute below

        // Section label
        if y < inner.bottom() {
            let label = Span::styled("Context Budget", style_system().add_modifier(Modifier::BOLD));
            buf.set_span(inner.x, y, &label, inner.width);
            y += 1;
        }

        // Token bar
        if y < inner.bottom() {
            let pct = if self.token_budget > 0 {
                (self.token_used as f64 / self.token_budget as f64).min(1.0)
            } else { 0.0 };
            let bar_w = inner.width.saturating_sub(2) as usize;
            let filled = (pct * bar_w as f64).round() as usize;
            let empty = bar_w.saturating_sub(filled);

            let bar_color = if pct > 0.9 {
                style_error().fg.unwrap_or(Color::Red)
            } else if pct > 0.7 {
                style_status_tool().fg.unwrap_or(Color::Yellow)
            } else {
                col_aqua()
            };

            let filled_str = "█".repeat(filled);
            let empty_str  = "░".repeat(empty);
            let line = Line::from(vec![
                Span::styled(filled_str, Style::default().fg(bar_color)),
                Span::styled(empty_str, style_separator()),
            ]);
            Paragraph::new(line).render(
                Rect { x: inner.x, y, width: inner.width, height: 1 },
                buf,
            );
            y += 1;
        }

        // Token count label — show raw count below 1 000 so it never displays as "0k"
        if y < inner.bottom() {
            let used_str = if self.token_used >= 1_000 {
                format!("{}k", self.token_used / 1_000)
            } else {
                self.token_used.to_string()
            };
            let budget_k = self.token_budget / 1_000;
            let label = format!("~{used_str} / {budget_k}k tokens");
            let span = Span::styled(label, style_system());
            buf.set_span(inner.x, y, &span, inner.width);
            y += 1;
        }

        // Token history line graph
        let chart_h = 6u16;
        if self.token_history.len() >= 2 && inner.bottom().saturating_sub(y) > chart_h + 2 {
            let data: Vec<(f64, f64)> = self.token_history.iter().enumerate()
                .map(|(i, &v)| (i as f64, v as f64))
                .collect();
            let x_max = (TOKEN_HISTORY_LEN - 1) as f64;
            let y_max = self.token_budget as f64;

            let dataset = Dataset::default()
                .graph_type(GraphType::Line)
                .marker(Marker::Braille)
                .style(Style::default().fg(col_aqua()))
                .data(&data);

            let chart = Chart::new(vec![dataset])
                .x_axis(Axis::default().bounds([0.0, x_max]))
                .y_axis(Axis::default().bounds([0.0, y_max]));

            chart.render(
                Rect { x: inner.x, y, width: inner.width, height: chart_h },
                buf,
            );
            y += chart_h;
        }

        // Divider
        if y < inner.bottom() {
            let div: String = "─".repeat(inner.width as usize);
            let span = Span::styled(div, style_separator());
            buf.set_span(inner.x, y, &span, inner.width);
            y += 1;
        }

        // ── Plan section (upfront checklist, only shown once one exists) ────
        if !self.plan.is_empty() {
            if y < inner.bottom() {
                let span = Span::styled("Plan", style_system().add_modifier(Modifier::BOLD));
                buf.set_span(inner.x, y, &span, inner.width);
                y += 1;
            }

            // Show the last N steps that fit (matches the Tasks section below) —
            // showing the first N instead would permanently hide whichever step
            // is currently InProgress/Failed once the plan overflows the sidebar.
            let available_rows = inner.bottom().saturating_sub(y) as usize;
            let start = self.plan.len().saturating_sub(available_rows);

            for step in self.plan[start..].iter() {
                if y >= inner.bottom() { break; }

                let (marker, marker_style) = match step.status {
                    TaskStatus::Completed  => ("x", style_success()),
                    TaskStatus::Failed     => ("!", style_error()),
                    TaskStatus::InProgress => (">", style_prompt_active()),
                    TaskStatus::Pending    => (" ", style_system()),
                };

                let max_desc = inner.width.saturating_sub(4) as usize;
                let desc = if step.description.chars().count() > max_desc {
                    step.description.chars().take(max_desc.saturating_sub(1)).collect::<String>() + "…"
                } else {
                    step.description.clone()
                };

                let line = Line::from(vec![
                    Span::styled("[", style_separator()),
                    Span::styled(marker, marker_style),
                    Span::styled("] ", style_separator()),
                    Span::styled(desc, style_inline_text()),
                ]);
                Paragraph::new(line).render(
                    Rect { x: inner.x, y, width: inner.width, height: 1 },
                    buf,
                );
                y += 1;
            }

            if y < inner.bottom() {
                let div: String = "─".repeat(inner.width as usize);
                let span = Span::styled(div, style_separator());
                buf.set_span(inner.x, y, &span, inner.width);
                y += 1;
            }
        }

        // ── Task list section ──────────────────────────────────────────────
        if y < inner.bottom() {
            let span = Span::styled("Tasks", style_system().add_modifier(Modifier::BOLD));
            buf.set_span(inner.x, y, &span, inner.width);
            y += 1;
        }

        if self.tasks.is_empty() {
            if y < inner.bottom() {
                let span = Span::styled("  (no active tasks)", style_placeholder());
                buf.set_span(inner.x, y, &span, inner.width);
                y += 1;
            }
        } else {
            // Show last N tasks that fit
            let available_rows = inner.bottom().saturating_sub(y) as usize;
            let start = self.tasks.len().saturating_sub(available_rows);

            for step in self.tasks[start..].iter() {
                if y >= inner.bottom() { break; }

                let (marker, marker_style) = match step.status {
                    TaskStatus::Completed  => ("x", style_success()),
                    TaskStatus::Failed     => ("!", style_error()),
                    TaskStatus::InProgress => (">", style_prompt_active()),
                    TaskStatus::Pending    => (" ", style_system()),
                };

                // Steps that ran concurrently (see Engine::execute_tools' parallel-safe
                // batching) get a "∥" hint before the bracket instead of a leading
                // space, so a group reads as "these ran together" at a glance.
                let (lead, max_desc) = if step.parallel_group.is_some() {
                    ("∥", inner.width.saturating_sub(5) as usize)
                } else {
                    (" ", inner.width.saturating_sub(4) as usize)
                };
                let desc = if step.description.chars().count() > max_desc {
                    step.description.chars().take(max_desc.saturating_sub(1)).collect::<String>() + "…"
                } else {
                    step.description.clone()
                };

                let line = Line::from(vec![
                    Span::styled(lead, style_separator()),
                    Span::styled("[", style_separator()),
                    Span::styled(marker, marker_style),
                    Span::styled("] ", style_separator()),
                    Span::styled(desc, style_inline_text()),
                ]);
                Paragraph::new(line).render(
                    Rect { x: inner.x, y, width: inner.width, height: 1 },
                    buf,
                );
                y += 1;
            }
        }

        // ── Subagents section ───────────────────────────────────────────────
        if y < inner.bottom() {
            let div: String = "─".repeat(inner.width as usize);
            let span = Span::styled(div, style_separator());
            buf.set_span(inner.x, y, &span, inner.width);
            y += 1;
        }

        if y < inner.bottom() {
            let span = Span::styled("Subagents", style_system().add_modifier(Modifier::BOLD));
            buf.set_span(inner.x, y, &span, inner.width);
            y += 1;
        }

        if self.subagents.is_empty() {
            if y < inner.bottom() {
                let span = Span::styled("  (none running)", style_placeholder());
                buf.set_span(inner.x, y, &span, inner.width);
            }
            return;
        }

        let available_rows = inner.bottom().saturating_sub(y) as usize;
        let start = self.subagents.len().saturating_sub(available_rows);

        for sa in self.subagents[start..].iter() {
            if y >= inner.bottom() { break; }

            let (marker, marker_style) = match sa.status {
                SubagentStatus::Running => (">", style_prompt_active()),
                SubagentStatus::Done => ("x", style_success()),
                SubagentStatus::Failed => ("!", style_error()),
            };

            let detail = match (&sa.status, &sa.current_tool) {
                (SubagentStatus::Running, Some(tool)) => format!("{}: {tool}", sa.label),
                _ => sa.label.clone(),
            };

            let max_desc = inner.width.saturating_sub(4) as usize;
            let desc = if detail.chars().count() > max_desc {
                detail.chars().take(max_desc.saturating_sub(1)).collect::<String>() + "…"
            } else {
                detail
            };

            let line = Line::from(vec![
                Span::styled("[", style_separator()),
                Span::styled(marker, marker_style),
                Span::styled("] ", style_separator()),
                Span::styled(desc, style_inline_text()),
            ]);
            Paragraph::new(line).render(
                Rect { x: inner.x, y, width: inner.width, height: 1 },
                buf,
            );
            y += 1;
        }
    }
}

// A sensible foreground color to pair with col_app_bg() in the sidebar.
fn fg_on_app_bg() -> Color {
    if is_light() { Color::Rgb(30, 50, 90) } else { Color::Rgb(180, 200, 230) }
}

