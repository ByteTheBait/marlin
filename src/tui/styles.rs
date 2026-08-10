// Style palette — not every constant or function is referenced by current UI code;
// unused items are intentionally kept as the palette grows.
#![allow(dead_code)]

use std::sync::{OnceLock, RwLock};
use std::sync::atomic::{AtomicBool, Ordering};

use ratatui::style::{Color, Modifier, Style};

use crate::config::{ThemeColors, ThemePalette};

static LIGHT_THEME: AtomicBool = AtomicBool::new(false);
// RwLock inside OnceLock so load_palette() can replace the palette at runtime
// (e.g. when the user runs /theme <name> to switch named themes).
static THEME: OnceLock<RwLock<ThemePalette>> = OnceLock::new();

fn palette_lock() -> &'static RwLock<ThemePalette> {
    THEME.get_or_init(|| RwLock::new(ThemePalette::default()))
}

pub fn set_light_theme(on: bool) {
    LIGHT_THEME.store(on, Ordering::Relaxed);
}

pub fn is_light() -> bool {
    LIGHT_THEME.load(Ordering::Relaxed)
}

/// Apply a palette loaded from `~/.marlin/theme.toml` or a named theme file.
/// Safe to call at any time — takes effect on the next render frame.
pub fn load_palette(p: ThemePalette) {
    *palette_lock().write().unwrap() = p;
}

// Resolve a named color from the active palette, falling back to built-in defaults.
fn theme_rgb(
    dark_default: [u8; 3],
    light_default: [u8; 3],
    key: fn(&ThemeColors) -> Option<[u8; 3]>,
) -> Color {
    let lock = palette_lock().read().unwrap();
    if is_light() {
        let c = lock.light.as_ref().and_then(key);
        let [r, g, b] = c.unwrap_or(light_default);
        Color::Rgb(r, g, b)
    } else {
        let c = lock.dark.as_ref().and_then(key);
        let [r, g, b] = c.unwrap_or(dark_default);
        Color::Rgb(r, g, b)
    }
}

// ── Semantic color accessors ─────────────────────────────────────────────────

pub fn col_aqua() -> Color {
    theme_rgb([0, 200, 200], [0, 110, 140], |t| t.assistant)
}
pub fn col_cobalt() -> Color {
    theme_rgb([40, 90, 210], [25, 70, 165], |t| t.cobalt)
}
pub fn col_steel() -> Color {
    theme_rgb([90, 120, 155], [75, 105, 145], |t| t.steel)
}
pub fn col_system() -> Color {
    theme_rgb([100, 125, 150], [95, 120, 150], |t| t.system)
}
pub fn col_user() -> Color {
    theme_rgb([200, 215, 245], [20, 45, 95], |t| t.user)
}
pub fn col_success() -> Color {
    theme_rgb([70, 195, 110], [25, 125, 60], |t| t.success)
}
pub fn col_error() -> Color {
    theme_rgb([215, 70, 70], [175, 35, 35], |t| t.error)
}
pub fn col_amber() -> Color {
    theme_rgb([215, 155, 45], [150, 90, 15], |t| t.amber)
}
fn col_bg_status() -> Color {
    if is_light() { Color::Rgb(215, 230, 248) } else { Color::Rgb(20, 30, 68) }
}
fn col_deep_ocean() -> Color {
    // Foreground on colored chips — white in both themes
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

/// Background fill of the tool-call badge chip.
pub fn style_tool_badge() -> Style {
    if is_light() {
        Style::default()
            .fg(Color::Rgb(25, 60, 120))
            .bg(Color::Rgb(200, 220, 250))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Rgb(160, 205, 255))
            .bg(Color::Rgb(18, 38, 80))
            .add_modifier(Modifier::BOLD)
    }
}

/// Color of the badge bracket glyphs (╭ and ╮).
pub fn style_tool_badge_bracket() -> Style {
    if is_light() {
        Style::default().fg(Color::Rgb(140, 175, 220))
    } else {
        Style::default().fg(Color::Rgb(35, 65, 120))
    }
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

pub fn style_inline_italic() -> Style {
    style_inline_text().add_modifier(Modifier::ITALIC)
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

/// Prompt-injection-over-budget badge — amber background, never blocking.
pub fn style_status_budget_warn() -> Style {
    Style::default()
        .fg(Color::Rgb(30, 20, 0))
        .bg(if is_light() { Color::Rgb(210, 150, 40) } else { Color::Rgb(230, 175, 60) })
        .add_modifier(Modifier::BOLD)
}

pub fn style_user_text() -> Style {
    Style::default().fg(col_user())
}

pub fn style_thinking() -> Style {
    Style::default().fg(col_system()).add_modifier(Modifier::ITALIC | Modifier::DIM)
}

pub fn col_app_bg() -> Color {
    theme_rgb([8, 12, 24], [252, 253, 255], |t| t.bg)
}

pub fn style_app_bg() -> Style {
    Style::default().bg(col_app_bg())
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
