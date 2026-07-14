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
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }
    let dangerously_skip_permissions = args.iter().any(|a| a == "--dangerously-skip-permissions");

    let mut cfg = config::Config::load()?;
    if dangerously_skip_permissions {
        // Session-only override — deliberately not persisted to config.json,
        // same as Claude Code's own flag of the same name. Bypasses every
        // approval prompt (destructive commands, path escapes) via the
        // preflight funnel; see preflight::check.
        cfg.skip_permissions = true;
        eprintln!(
            "marlin: --dangerously-skip-permissions is set — all permission \
             checks are bypassed for this session."
        );
    }
    let marlin_dir = config::marlin_dir()?;

    tui::styles::set_light_theme(cfg.theme == "light");
    tui::styles::load_palette(config::load_theme(&marlin_dir));
    let layout = config::load_layout(&marlin_dir);

    let mut eng = engine::Engine::new(cfg)?;

    // Print preflight diagnostics to the real terminal before the TUI takes over
    // the alternate screen — missing binaries, unparsable config files, skill
    // validation issues, a stale index. Informational only, never blocks startup.
    let diagnostics = eng.startup_diagnostics();
    if !diagnostics.is_empty() {
        eprintln!("marlin: preflight startup ({} note(s)):", diagnostics.len());
        for line in diagnostics {
            eprintln!("  {line}");
        }
        eprintln!();
    }

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

fn print_help() {
    println!(
        "marlin\n\n\
         Usage: marlin [options]\n\n\
         Options:\n  \
         --dangerously-skip-permissions  Skip all permission checks for this session\n                                   \
         (destructive commands, path escapes) without prompting.\n                                   \
         Not persisted — set /permissions skip in-app to persist.\n  \
         -h, --help                      Show this help\n"
    );
}
