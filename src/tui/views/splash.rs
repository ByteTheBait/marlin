use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};
use tachyonfx::{
    fx, Effect, EffectRenderer, EffectTimer, Interpolatable, Interpolation,
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
        let bg = col_app_bg();

        // Radial reveal: outer edge first, converging to center
        let effect = fx::effect_fn(
            (),
            EffectTimer::from_ms(1400, Interpolation::CubicOut),
            move |_state, ctx, cell_iter| {
                let alpha = ctx.alpha();
                let area = ctx.area;

                let cx = area.x as f32 + area.width as f32 / 2.0;
                let cy = area.y as f32 + area.height as f32 / 2.0;

                // Aspect-ratio correction: terminal cells are ~2× taller than wide,
                // so scale the y-axis to get a circle instead of an oval.
                let hw = area.width as f32 / 2.0;
                let hh = area.height as f32; // doubled via *2 below
                let max_d = (hw * hw + hh * hh).sqrt();

                // reveal_d shrinks from max_d → 0 as alpha goes 0 → 1.
                // Cells closer than reveal_d to center are still hidden.
                let reveal_d = max_d * (1.0 - alpha);
                let band = 5.0f32; // soft gradient ring width in distance units

                cell_iter.for_each_cell(|pos, cell| {
                    let dx = pos.x as f32 - cx;
                    let dy = (pos.y as f32 - cy) * 2.0;
                    let d = (dx * dx + dy * dy).sqrt();

                    if d < reveal_d - band {
                        // fully veiled
                        cell.set_fg(bg);
                        cell.set_bg(bg);
                        cell.set_char(' ');
                    } else if d < reveal_d {
                        // soft gradient ring blending into the veil
                        let t = (reveal_d - d) / band;
                        let new_fg = cell.fg.lerp(&bg, t);
                        let new_bg = cell.bg.lerp(&bg, t);
                        cell.set_fg(new_fg);
                        cell.set_bg(new_bg);
                    }
                    // else: d >= reveal_d → fully visible, leave as-is
                });
            },
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
