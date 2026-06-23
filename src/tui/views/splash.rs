use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};
use tachyonfx::{
    fx, Effect, EffectRenderer, EffectTimer, Interpolation, Motion,
    Duration as FxDuration,
};

use crate::tui::styles::*;

const VISIBLE_MS: u64 = 2400; // how long before auto-transition

pub struct SplashView {
    start: Instant,
    last_tick: Instant,
    effect: Effect,
}

impl SplashView {
    pub fn new() -> Self {
        // sweep down from top, revealing content row by row
        let effect = fx::sweep_in(
            Motion::UpToDown,
            8,            // gradient length (cols)
            3,            // randomness
            Color::Black,
            EffectTimer::from_ms(1400, Interpolation::CubicOut),
        );
        Self {
            start: Instant::now(),
            last_tick: Instant::now(),
            effect,
        }
    }

    pub fn is_done(&self) -> bool {
        self.start.elapsed() >= Duration::from_millis(VISIBLE_MS)
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        // Static content — tachyonfx drives the reveal animation
        let lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "><(((o>",
                Style::default().fg(COL_AQUA),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "m a r l i n",
                Style::default()
                    .fg(Color::Rgb(190, 210, 255))
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "ai coding assistant",
                Style::default().fg(COL_SYSTEM),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "v0.1.0  -  rust edition",
                Style::default().fg(Color::Rgb(50, 65, 85)),
            )),
        ];

        let total_h = lines.len() as u16;
        let vert_pad = area.height.saturating_sub(total_h + 4) / 2;
        let inner = Rect {
            y: area.y + vert_pad,
            height: area.height.saturating_sub(vert_pad),
            ..area
        };

        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .render(inner, buf);

        // "press any key" hint — faint, near bottom
        let hint_y = area.bottom().saturating_sub(2);
        if hint_y > area.y {
            Paragraph::new(Line::from(Span::styled(
                "press any key",
                Style::default().fg(Color::Rgb(32, 46, 65)),
            )))
            .alignment(Alignment::Center)
            .render(Rect { y: hint_y, height: 1, ..area }, buf);
        }

        // Drive the tachyonfx effect — must happen after widget renders
        let tick_ms = self.last_tick.elapsed().as_millis().min(100) as u32;
        self.last_tick = Instant::now();
        buf.render_effect(&mut self.effect, area, FxDuration::from_millis(tick_ms));
    }
}
