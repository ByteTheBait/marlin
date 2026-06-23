use std::time::{Duration, Instant};

use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
    buffer::Buffer,
};

use crate::tui::styles::*;

const SPLASH_DURATION: Duration = Duration::from_millis(1800);

fn fade(r: u8, g: u8, b: u8, t: f64) -> Color {
    let t = t.clamp(0.0, 1.0);
    Color::Rgb(
        (r as f64 * t) as u8,
        (g as f64 * t) as u8,
        (b as f64 * t) as u8,
    )
}

fn ease_in(t: f64) -> f64 { t * t }

pub struct SplashView {
    start: Instant,
}

impl SplashView {
    pub fn new() -> Self {
        Self { start: Instant::now() }
    }

    pub fn is_done(&self) -> bool {
        self.start.elapsed() >= SPLASH_DURATION
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        let elapsed = self.start.elapsed().as_millis() as f64;
        let total = SPLASH_DURATION.as_millis() as f64;
        let p = (elapsed / total).clamp(0.0, 1.0);

        // Staggered fade-in: fish → name → subtitle → version
        let fish_t = ease_in(((p - 0.00) / 0.40).clamp(0.0, 1.0));
        let name_t = ease_in(((p - 0.30) / 0.40).clamp(0.0, 1.0));
        let sub_t  = ease_in(((p - 0.60) / 0.40).clamp(0.0, 1.0));
        let ver_t  = ease_in(((p - 0.75) / 0.25).clamp(0.0, 1.0));

        let lines: Vec<Line> = vec![
            Line::from(Span::styled(
                "><(((o>",
                Style::default().fg(fade(0, 200, 200, fish_t)),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "m a r l i n",
                Style::default().fg(fade(190, 210, 255, name_t)).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "ai coding assistant",
                Style::default().fg(fade(100, 125, 150, sub_t)),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "v0.1.0  -  rust edition",
                Style::default().fg(fade(55, 70, 90, ver_t)),
            )),
        ];

        let total_h = lines.len() as u16;
        let vert_pad = area.height.saturating_sub(total_h).saturating_sub(2) / 2;

        let inner = Rect {
            y: area.y + vert_pad,
            height: area.height.saturating_sub(vert_pad),
            ..area
        };

        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .render(inner, buf);

        // "press any key" hint fades in near the end
        if ver_t > 0.4 {
            let hint_alpha = ((ver_t - 0.4) / 0.6).clamp(0.0, 1.0);
            let hint_y = area.bottom().saturating_sub(2);
            if hint_y > area.y {
                let hint_rect = Rect { y: hint_y, height: 1, ..area };
                Paragraph::new(Line::from(Span::styled(
                    "press any key",
                    Style::default().fg(fade(45, 60, 80, hint_alpha)),
                )))
                .alignment(Alignment::Center)
                .render(hint_rect, buf);
            }
        }
    }
}
