use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use marlin_engine::Action;
use crate::widgets::viewer::ViewerPane;

use super::ChatView;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_c() -> KeyEvent {
    KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL)
}

#[test]
fn esc_twice_with_nothing_running_quits() {
    let mut chat = ChatView::new(80, 24);
    assert!(chat.on_key(key(KeyCode::Esc)).is_none());
    assert!(matches!(chat.on_key(key(KeyCode::Esc)), Some(Action::Quit)));
}

#[test]
fn ctrl_c_twice_with_nothing_running_quits() {
    let mut chat = ChatView::new(80, 24);
    assert!(chat.on_key(ctrl_c()).is_none());
    assert!(matches!(chat.on_key(ctrl_c()), Some(Action::Quit)));
}

#[test]
fn mixing_esc_and_ctrl_c_still_counts_as_two_in_a_row() {
    let mut chat = ChatView::new(80, 24);
    assert!(chat.on_key(ctrl_c()).is_none());
    assert!(matches!(chat.on_key(key(KeyCode::Esc)), Some(Action::Quit)));
}

#[test]
fn an_unrelated_key_in_between_disarms_quit() {
    let mut chat = ChatView::new(80, 24);
    assert!(chat.on_key(ctrl_c()).is_none());
    chat.on_key(key(KeyCode::Char('a')));
    // Second Ctrl+C now only re-arms — it should NOT quit immediately.
    assert!(chat.on_key(ctrl_c()).is_none());
}

#[test]
fn cancel_while_streaming_cancels_instead_of_arming_quit() {
    let mut chat = ChatView::new(80, 24);
    chat.streaming = true;
    assert!(matches!(chat.on_key(ctrl_c()), Some(Action::CancelStream)));
    // Streaming is now off; a second Ctrl+C should just arm quit, not fire it.
    assert!(chat.on_key(ctrl_c()).is_none());
    assert!(matches!(chat.on_key(ctrl_c()), Some(Action::Quit)));
}

#[test]
fn mouse_scroll_up_in_base_state_scrolls_viewport() {
    let mut chat = ChatView::new(80, 24);
    chat.at_bottom = true;
    assert!(chat.on_mouse_scroll(true).is_none());
    assert!(!chat.at_bottom);
}

#[test]
fn mouse_scroll_in_base_state_never_triggers_input_history_nav() {
    let mut chat = ChatView::new(80, 24);
    chat.input_history = vec!["previous command".to_string()];
    chat.history_idx = -1;
    // Preconditions for a literal ↑ keypress to trigger history-nav are
    // met (empty single-line textarea, history available) — a wheel
    // notch must scroll the viewport instead, never touch input history.
    chat.on_mouse_scroll(true);
    assert_eq!(chat.history_idx, -1);
    assert!(chat
        .textarea
        .lines()
        .first()
        .map(String::as_str)
        .unwrap_or("")
        .is_empty());
}

#[test]
fn mouse_scroll_routes_through_overlay_when_one_is_open() {
    let mut chat = ChatView::new(80, 24);
    chat.viewer = Some(ViewerPane::new(
        "f.txt".into(),
        (1..=50)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n"),
    ));
    // Routed into the viewer pane's own on_key (Up/Down), not viewport
    // scroll or input-history nav — ViewerOutcome::None means no Action.
    assert!(chat.on_mouse_scroll(false).is_none());
    assert!(chat.viewer.is_some(), "scrolling shouldn't close the pane");
}
