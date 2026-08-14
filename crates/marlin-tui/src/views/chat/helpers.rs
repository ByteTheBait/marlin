use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::styles::*;

pub(super) fn rate_bar(pct: f64, width: usize) -> String {
    let filled = (pct * width as f64) as usize;
    let empty = width.saturating_sub(filled);
    format!("[{}{}]", "█".repeat(filled), "░".repeat(empty))
}

pub(super) fn build_tool_bubble(
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

pub(super) fn tool_display_name(raw: &str) -> &'static str {
    match raw {
        "read_file" => "Read",
        "write_file" => "Write",
        "edit_file" => "Update",
        "multi_edit" => "Multi-update",
        "notebook_edit" => "Notebook",
        "run_command" => "Run",
        "list_directory" => "List",
        "create_directory" => "Mkdir",
        "search_codebase" => "Search",
        "run_skill" => "Skill",
        "ast_skeleton" => "Skeleton",
        "ast_get_node" => "Node",
        "ast_mutate" => "Mutate",
        _ => "Tool",
    }
}
