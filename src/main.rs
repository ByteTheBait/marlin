mod commands;
mod config;
mod engine;
mod history;
mod index;
mod preflight;
mod providers;
mod skills;
mod snapshots;
mod tools;
mod tui;

use anyhow::Result;
use tokio::sync::mpsc;

fn main() -> Result<()> {
    let cfg = config::Config::load()?;
    let marlin_dir = config::marlin_dir()?;

    tui::styles::set_light_theme(cfg.theme == "light");
    tui::styles::load_palette(config::load_theme(&marlin_dir));
    let layout = config::load_layout(&marlin_dir);

    let mut eng = engine::Engine::new(cfg)?;

    let (action_tx, action_rx) = mpsc::channel::<engine::Action>(64);
    let (ui_tx, ui_rx) = mpsc::channel::<engine::UiUpdate>(256);

    // Spawn async engine on a Tokio multi-thread runtime
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.spawn(async move {
        eng.run(action_rx, ui_tx).await;
    });

    // TUI runs on the main thread (synchronous)
    tui::runner::run(action_tx, ui_rx, layout)?;

    Ok(())
}
