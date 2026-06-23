use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use crate::engine::{Action, UiUpdate};
use crate::tui::{
    views::{chat::ChatView, splash::SplashView},
    widgets::statusbar::StatusBar,
};

enum View {
    Splash(SplashView),
    Chat,
}

pub fn run(
    action_tx: mpsc::Sender<Action>,
    mut ui_rx: mpsc::Receiver<UiUpdate>,
) -> io::Result<()> {
    terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let size = terminal.size()?;
    let mut status_bar = StatusBar::new(size.width);
    let mut chat = ChatView::new(size.width, size.height.saturating_sub(1));
    let mut view = View::Splash(SplashView::new());

    // Rate-limit countdown ticker
    let mut rate_tick = std::time::Instant::now();

    'outer: loop {
        // Process all pending engine updates
        loop {
            match ui_rx.try_recv() {
                Ok(update) => {
                    match &update {
                        UiUpdate::StatusUpdate(info) => {
                            status_bar.provider = info.provider.clone();
                            status_bar.model = info.model.clone();
                        }
                        UiUpdate::ToolCall { name, .. } => {
                            status_bar.active_tool = name.clone();
                            status_bar.streaming = true;
                        }
                        UiUpdate::StreamChunk(_) => {
                            status_bar.streaming = true;
                        }
                        UiUpdate::GoalComplete { .. } => {
                            status_bar.active_tool.clear();
                            status_bar.streaming = false;
                        }
                        _ => {}
                    }
                    chat.apply_update(update);
                }
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break 'outer,
            }
        }

        // Splash auto-transition
        if let View::Splash(ref splash) = view {
            if splash.is_done() {
                view = View::Chat;
                status_bar.mode = "chat".into();
            }
        }

        // Rate-limit countdown
        if chat.rate_limited && rate_tick.elapsed() >= Duration::from_secs(1) {
            rate_tick = std::time::Instant::now();
            if chat.tick_rate_limit() && !chat.rate_limited {
                // Resume action is handled by the engine sleeping itself,
                // but we add a visual cue
                chat.add_system("Rate limit cleared — resuming...");
            }
        }

        // Render
        terminal.draw(|f| {
            let area = f.area();
            let buf = f.buffer_mut();

            match &mut view {
                View::Splash(splash) => {
                    splash.render(area, buf);
                }
                View::Chat => {
                    let status_area = ratatui::layout::Rect {
                        y: area.y,
                        height: 1,
                        ..area
                    };
                    let chat_area = ratatui::layout::Rect {
                        y: area.y + 1,
                        height: area.height.saturating_sub(1),
                        ..area
                    };
                    status_bar.streaming = chat.streaming;
                    status_bar.render(status_area, buf);
                    chat.render(chat_area, buf);
                }
            }
        })?;

        // Poll for terminal events (16ms ≈ 60fps)
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => {
                    // Ctrl+Q always quits
                    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        let _ = action_tx.blocking_send(Action::Quit);
                        break;
                    }

                    // Splash just waits for any key to skip
                    if let View::Splash(_) = &view {
                        view = View::Chat;
                        status_bar.mode = "chat".into();
                        continue;
                    }

                    // Delegate to chat view
                    if let Some(action) = chat.on_key(key) {
                        match &action {
                            Action::Quit => {
                                let _ = action_tx.blocking_send(Action::Quit);
                                break;
                            }
                            _ => {
                                let _ = action_tx.blocking_send(action);
                            }
                        }
                    }
                }
                Event::Resize(w, h) => {
                    status_bar.width = w;
                    chat.resize(w, h.saturating_sub(1));
                }
                _ => {}
            }
        }
    }

    // Restore terminal
    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}
