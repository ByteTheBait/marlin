use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::style::{Color, Modifier, Style};

static LIGHT_THEME: AtomicBool = AtomicBool::new(false);

pub fn set_light_theme(on: bool) {
    LIGHT_THEME.store(on, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT_THEME.load(Ordering::Relaxed)
}

// ── Semantic color accessors ─────────────────────────────────────────────────

pub fn col_aqua() -> Color {
    if is_light() { Color::Rgb(0, 110, 140) } else { Color::Rgb(0, 200, 200) }
}
fn col_cobalt() -> Color {
    if is_light() { Color::Rgb(25, 70, 165) } else { Color::Rgb(40, 90, 210) }
}
fn col_steel() -> Color {
    if is_light() { Color::Rgb(75, 105, 145) } else { Color::Rgb(90, 120, 155) }
}
fn col_system() -> Color {
    if is_light() { Color::Rgb(95, 120, 150) } else { Color::Rgb(100, 125, 150) }
}
fn col_user() -> Color {
    if is_light() { Color::Rgb(20, 45, 95) } else { Color::Rgb(200, 215, 245) }
}
fn col_success() -> Color {
    if is_light() { Color::Rgb(25, 125, 60) } else { Color::Rgb(70, 195, 110) }
}
fn col_error() -> Color {
    if is_light() { Color::Rgb(175, 35, 35) } else { Color::Rgb(215, 70, 70) }
}
fn col_amber() -> Color {
    if is_light() { Color::Rgb(150, 90, 15) } else { Color::Rgb(215, 155, 45) }
}
fn col_bg_status() -> Color {
    if is_light() { Color::Rgb(215, 230, 248) } else { Color::Rgb(20, 30, 68) }
}
fn col_deep_ocean() -> Color {
    // Used as foreground on coloured chips — white in both themes
    if is_light() { Color::Rgb(255, 255, 255) } else { Color::Rgb(8, 12, 24) }
}

// Keep as pub consts for the few places that still need them directly
pub const COL_DEEP_OCEAN: Color  = Color::Rgb(8, 12, 24);
pub const COL_COBALT: Color      = Color::Rgb(40, 90, 210);
pub const COL_AQUA: Color        = Color::Rgb(0, 200, 200);
pub const COL_STEEL: Color       = Color::Rgb(90, 120, 155);
pub const COL_SUCCESS: Color     = Color::Rgb(70, 195, 110);
pub const COL_ERROR: Color       = Color::Rgb(215, 70, 70);
pub const COL_SYSTEM: Color      = Color::Rgb(100, 125, 150);
pub const COL_USER: Color        = Color::Rgb(200, 215, 245);
pub const COL_ASSISTANT: Color   = Color::Rgb(0, 200, 200);
pub const COL_AMBER: Color       = Color::Rgb(215, 155, 45);
pub const COL_BG_STATUS: Color   = Color::Rgb(20, 30, 68);

// ── Named style functions (theme-aware) ──────────────────────────────────────

pub fn style_system() -> Style {
    Style::default().fg(col_system())
}

pub fn style_error() -> Style {
    Style::default().fg(col_error())
}

pub fn style_success() -> Style {
    Style::default().fg(col_success())
}

pub fn style_user_label() -> Style {
    Style::default().fg(col_user()).add_modifier(Modifier::BOLD)
}

pub fn style_assistant_label() -> Style {
    Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD)
}

pub fn style_help_key() -> Style {
    Style::default().fg(col_cobalt()).add_modifier(Modifier::BOLD)
}

pub fn style_tool_icon() -> Style {
    Style::default().fg(col_amber())
}

pub fn style_tool_name() -> Style {
    Style::default().fg(col_cobalt()).add_modifier(Modifier::BOLD)
}

pub fn style_input_border_active() -> Style {
    Style::default().fg(col_aqua())
}

pub fn style_input_border_inactive() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(180, 205, 235))
    } else {
        Style::default().fg(Color::Rgb(35, 55, 90))
    }
}

// ── Additional theme-aware styles used inline in chat.rs / statusbar.rs ─────

pub fn style_separator() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(195, 215, 238))
    } else {
        Style::default().fg(Color::Rgb(28, 42, 68))
    }
}

pub fn style_placeholder() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(165, 185, 215))
    } else {
        Style::default().fg(Color::Rgb(48, 62, 82))
    }
}

pub fn style_prompt_empty() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(170, 195, 230))
    } else {
        Style::default().fg(Color::Rgb(50, 70, 100))
    }
}

pub fn style_prompt_active() -> Style {
    Style::default().fg(col_aqua())
}

pub fn style_cursor() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(0, 115, 150))
    } else {
        Style::default().fg(Color::Rgb(8, 12, 24)).bg(Color::Rgb(0, 200, 200))
    }
}

pub fn style_code_block() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(50, 90, 30))
    } else {
        Style::default().fg(Color::Rgb(200, 220, 160))
    }
}

pub fn style_inline_text() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(25, 45, 80))
    } else {
        Style::default().fg(Color::Rgb(220, 230, 240))
    }
}

pub fn style_inline_bold() -> Style {
    Style::default().fg(col_user()).add_modifier(Modifier::BOLD)
}

pub fn style_inline_code() -> Style {
    style_code_block()
}

pub fn style_bubble_border() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(185, 210, 238))
    } else {
        Style::default().fg(Color::Rgb(25, 45, 80))
    }
}

pub fn style_bubble_dots() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(0, 115, 145))
    } else {
        Style::default().fg(Color::Rgb(0, 155, 155))
    }
}

pub fn style_suggestion_border() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(175, 205, 238))
    } else {
        Style::default().fg(Color::Rgb(30, 55, 100))
    }
}

pub fn style_suggestion_desc() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(105, 135, 170))
    } else {
        Style::default().fg(Color::Rgb(65, 85, 105))
    }
}

pub fn style_suggestion_cmd() -> Style {
    Style::default().fg(col_cobalt())
}

pub fn style_suggestion_cmd_exact() -> Style {
    Style::default().fg(col_aqua()).add_modifier(Modifier::BOLD)
}

pub fn style_suggestion_args() -> Style {
    Style::default().fg(col_steel())
}

pub fn style_status_bg() -> Style {
    Style::default().bg(col_bg_status())
}

pub fn style_status_chip() -> Style {
    Style::default()
        .fg(col_deep_ocean())
        .bg(col_cobalt())
        .add_modifier(Modifier::BOLD)
}

pub fn style_status_provider() -> Style {
    Style::default().fg(col_aqua())
}

pub fn style_status_model() -> Style {
    Style::default().fg(col_steel())
}

pub fn style_status_tool() -> Style {
    Style::default().fg(col_amber())
}

pub fn style_status_tool_name() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(120, 70, 10))
    } else {
        Style::default().fg(Color::Rgb(175, 130, 40))
    }
}

pub fn style_status_streaming() -> Style {
    Style::default().fg(col_aqua())
}

/// SEXPR badge — blue background
pub fn style_status_ast_sexpr() -> Style {
    Style::default()
        .fg(col_deep_ocean())
        .bg(col_cobalt())
        .add_modifier(Modifier::BOLD)
}

/// HARNESS badge — magenta background
pub fn style_status_ast_harness() -> Style {
    Style::default()
        .fg(Color::Rgb(255, 255, 255))
        .bg(if is_light() { Color::Rgb(130, 30, 140) } else { Color::Rgb(190, 60, 210) })
        .add_modifier(Modifier::BOLD)
}

pub fn style_user_text() -> Style {
    Style::default().fg(col_user())
}

pub fn style_app_bg() -> Style {
    if is_light() {
        Style::default().bg(Color::Rgb(252, 253, 255))
    } else {
        Style::default().bg(Color::Rgb(8, 12, 24))
    }
}

pub fn col_app_bg() -> Color {
    if is_light() { Color::Rgb(252, 253, 255) } else { Color::Rgb(8, 12, 24) }
}

pub fn col_veil() -> Color {
    if is_light() { Color::Rgb(8, 12, 24) } else { Color::Rgb(252, 253, 255) }
}

pub fn style_input_bubble() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(180, 205, 235))
    } else {
        Style::default().fg(Color::Rgb(35, 55, 90))
    }
}
