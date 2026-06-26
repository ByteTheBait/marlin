use std::io;
use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget},
    Terminal,
};
use tokio::sync::mpsc;

use crate::config::AstMode;
use crate::engine::{Action, UiUpdate};
use crate::engine::tasks::TaskStep;
use crate::tui::{
    styles::{self, *},
    views::{chat::ChatView, splash::SplashView},
    widgets::{sidebar::Sidebar, statusbar::StatusBar},
};

const SIDEBAR_WIDTH: u16 = 34;
const MIN_WIDTH_FOR_SIDEBAR: u16 = 100;

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
    let mut sidebar = Sidebar::new();
    let mut view = View::Splash(SplashView::new());

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
                        UiUpdate::AstMode(mode) => {
                            status_bar.ast_mode = mode.clone();
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
                        UiUpdate::TaskUpdate(steps) => {
                            sidebar.tasks = steps.clone();
                        }
                        UiUpdate::TokenUsage { used, budget } => {
                            sidebar.token_used = *used;
                            sidebar.token_budget = *budget;
                            sidebar.push_token_sample(*used);
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
                chat.add_system("Rate limit cleared — resuming...");
            }
        }

        // Snapshot sidebar state for rendering
        let sidebar_snap = Sidebar {
            token_used: sidebar.token_used,
            token_budget: sidebar.token_budget,
            token_history: sidebar.token_history.clone(),
            tasks: sidebar.tasks.clone(),
        };
        let approval_cmd = chat.approval_pending.clone();

        // Render
        terminal.draw(|f| {
            let area = f.area();
            let buf = f.buffer_mut();

            // Fill background
            Block::default()
                .style(styles::style_app_bg())
                .render(area, buf);

            match &mut view {
                View::Splash(splash) => {
                    splash.render(area, buf);
                }
                View::Chat => {
                    let status_area = Rect { y: area.y, height: 1, ..area };
                    let body_area = Rect {
                        y: area.y + 1,
                        height: area.height.saturating_sub(1),
                        ..area
                    };
                    status_bar.streaming = chat.streaming;
                    status_bar.render(status_area, buf);

                    // Split body into chat + optional sidebar
                    let (chat_area, sidebar_area) = if area.width >= MIN_WIDTH_FOR_SIDEBAR {
                        let chunks = Layout::default()
                            .direction(Direction::Horizontal)
                            .constraints([
                                Constraint::Min(40),
                                Constraint::Length(SIDEBAR_WIDTH),
                            ])
                            .split(body_area);
                        (chunks[0], Some(chunks[1]))
                    } else {
                        (body_area, None)
                    };

                    chat.render(chat_area, buf);

                    if let Some(sa) = sidebar_area {
                        sidebar_snap.render(sa, buf);
                    }

                    // Approval modal — rendered on top of chat
                    if let Some(cmd) = &approval_cmd {
                        render_approval_modal(cmd, chat_area, buf);
                    }
                }
            }
        })?;

        // Poll for terminal events (16ms ≈ 60fps)
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        let _ = action_tx.blocking_send(Action::Quit);
                        break;
                    }

                    if let View::Splash(_) = &view {
                        view = View::Chat;
                        status_bar.mode = "chat".into();
                        continue;
                    }

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

    terminal::disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

fn render_approval_modal(cmd: &str, area: Rect, buf: &mut Buffer) {
    let modal_w = (area.width.min(72)).max(40);
    let modal_h = 7u16;
    let x = area.x + (area.width.saturating_sub(modal_w)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_h)) / 2;
    let modal_area = Rect { x, y, width: modal_w, height: modal_h };

    // Clear the background
    Clear.render(modal_area, buf);

    // Red-bordered block
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .border_style(Style::default().fg(Color::Rgb(215, 50, 50)).add_modifier(Modifier::BOLD))
        .title(Span::styled(
            " ⚠  Destructive Command ",
            Style::default().fg(Color::Rgb(215, 50, 50)).add_modifier(Modifier::BOLD),
        ));

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    let max_cmd = inner.width.saturating_sub(2) as usize;
    let cmd_display = if cmd.chars().count() > max_cmd {
        cmd.chars().take(max_cmd.saturating_sub(1)).collect::<String>() + "…"
    } else {
        cmd.to_string()
    };

    let lines = vec![
        Line::from(Span::styled(&cmd_display, Style::default().fg(Color::Rgb(240, 180, 60)).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled(
            "Allow this command to run?",
            Style::default().fg(Color::Rgb(210, 220, 240)),
        )),
        Line::from(vec![
            Span::styled("  [y] ", Style::default().fg(Color::Rgb(70, 195, 110)).add_modifier(Modifier::BOLD)),
            Span::styled("Yes, run it", Style::default().fg(Color::Rgb(150, 220, 170))),
            Span::styled("     [n] ", Style::default().fg(Color::Rgb(215, 70, 70)).add_modifier(Modifier::BOLD)),
            Span::styled("No, deny", Style::default().fg(Color::Rgb(220, 150, 150))),
        ]),
    ];

    Paragraph::new(lines)
        .style(Style::default().bg(Color::Rgb(12, 16, 30)))
        .render(inner, buf);
}
