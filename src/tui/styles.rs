use ratatui::style::{Color, Modifier, Style};

pub const COL_DEEP_OCEAN: Color = Color::Rgb(8, 12, 24);
pub const COL_COBALT: Color = Color::Rgb(40, 90, 210);
pub const COL_AQUA: Color = Color::Rgb(0, 200, 200);
pub const COL_STEEL: Color = Color::Rgb(90, 120, 155);
pub const COL_SUCCESS: Color = Color::Rgb(70, 195, 110);
pub const COL_ERROR: Color = Color::Rgb(215, 70, 70);
pub const COL_SYSTEM: Color = Color::Rgb(100, 125, 150);
pub const COL_USER: Color = Color::Rgb(200, 215, 245);
pub const COL_ASSISTANT: Color = Color::Rgb(0, 200, 200);
pub const COL_AMBER: Color = Color::Rgb(215, 155, 45);
pub const COL_BG_STATUS: Color = Color::Rgb(14, 20, 38);

pub fn style_system() -> Style {
    Style::default().fg(COL_SYSTEM)
}

pub fn style_error() -> Style {
    Style::default().fg(COL_ERROR)
}

pub fn style_success() -> Style {
    Style::default().fg(COL_SUCCESS)
}

pub fn style_user_label() -> Style {
    Style::default().fg(COL_USER).add_modifier(Modifier::BOLD)
}

pub fn style_assistant_label() -> Style {
    Style::default().fg(COL_AQUA).add_modifier(Modifier::BOLD)
}

pub fn style_help_key() -> Style {
    Style::default().fg(COL_COBALT).add_modifier(Modifier::BOLD)
}

pub fn style_tool_icon() -> Style {
    Style::default().fg(COL_AMBER)
}

pub fn style_tool_name() -> Style {
    Style::default().fg(COL_COBALT).add_modifier(Modifier::BOLD)
}

pub fn style_input_border_active() -> Style {
    Style::default().fg(COL_AQUA)
}

pub fn style_input_border_inactive() -> Style {
    Style::default().fg(Color::Rgb(35, 55, 90))
}
