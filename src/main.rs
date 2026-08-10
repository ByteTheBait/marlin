mod commands;
mod config;
mod engine;
mod history;
mod index;
mod mcp;
mod preflight;
mod providers;
mod skills;
mod snapshots;
mod tools;
mod tui;

use anyhow::Result;
use tokio::sync::mpsc;

/// Value that follows a `--flag <value>` pair in `args`, if present.
fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }
    let dangerously_skip_permissions = args.iter().any(|a| a == "--dangerously-skip-permissions");
    let run_prompt = flag_value(&args, "--run");

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

    if let Some(prompt) = run_prompt {
        // Headless mode: no TUI, no interactive approval loop, so a run that
        // could hit AwaitingApproval would hang forever. Require the same
        // bypass an interactive session would otherwise answer by hand.
        if !dangerously_skip_permissions {
            eprintln!(
                "marlin: --run requires --dangerously-skip-permissions — a headless run has \
                 nothing to answer an approval prompt with."
            );
            std::process::exit(1);
        }
        if let Some(cwd) = flag_value(&args, "--cwd") {
            cfg.work_dir = cwd;
        }
        if let Some(key) = flag_value(&args, "--api-key") {
            cfg.providers.entry(cfg.active_provider.clone()).or_default().api_key = key;
        }
        let output_path = flag_value(&args, "--output");

        let eng = engine::Engine::new(cfg)?;
        let exit_code = run_headless(eng, prompt, output_path)?;
        std::process::exit(exit_code);
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

/// Drives the engine headlessly for a single turn: sends `prompt` as the one user message,
/// accumulates streamed text until the turn completes or fails, writes the result to
/// `output_path` (or stdout if unset), and returns the process exit code.
///
/// Mirrors how `tui::runner::run` drives the same channels on the main thread — spawn the
/// engine on the Tokio runtime, then talk to it synchronously via blocking channel ops.
fn run_headless(mut eng: engine::Engine, prompt: String, output_path: Option<String>) -> Result<i32> {
    use engine::{Action, UiUpdate};

    let (action_tx, action_rx) = mpsc::channel::<Action>(64);
    let (ui_tx, mut ui_rx) = mpsc::channel::<UiUpdate>(256);

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.spawn(async move {
        eng.run(action_rx, ui_tx).await;
    });

    if action_tx.blocking_send(Action::SendMessage(prompt)).is_err() {
        eprintln!("marlin: engine channel closed before the run could start");
        return Ok(1);
    }

    let mut buf = String::new();
    let exit_code;
    loop {
        match ui_rx.blocking_recv() {
            Some(UiUpdate::StreamChunk(chunk)) => buf.push_str(&chunk),
            Some(UiUpdate::GoalComplete { .. }) => {
                exit_code = 0;
                break;
            }
            // Every ErrorMsg path that doesn't also send GoalComplete leaves the engine
            // parked back at its outer action loop, waiting for the next Action forever —
            // so this has to be treated as terminal rather than waiting for channel close.
            Some(UiUpdate::ErrorMsg(msg)) => {
                eprintln!("marlin: {msg}");
                exit_code = 1;
                break;
            }
            Some(_) => {} // tool calls, status updates, etc. — not turn-completion signals
            None => {
                eprintln!("marlin: engine exited without completing the turn");
                exit_code = 1;
                break;
            }
        }
    }

    let _ = action_tx.blocking_send(Action::Quit);

    let text = buf.trim();
    match output_path {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, text) {
                eprintln!("marlin: failed to write output to {path}: {e}");
                return Ok(1);
            }
        }
        None => println!("{text}"),
    }

    Ok(exit_code)
}

fn print_help() {
    println!(
        "marlin\n\n\
         Usage: marlin [options]\n\
         Usage: marlin --run <prompt> --dangerously-skip-permissions [--cwd <dir>] \
         [--output <path>] [--api-key <key>]\n\n\
         Options:\n  \
         --dangerously-skip-permissions  Skip all permission checks for this session\n                                   \
         (destructive commands, path escapes) without prompting.\n                                   \
         Not persisted — set /permissions skip in-app to persist.\n  \
         --run <prompt>                  Headless mode: run one prompt to completion with no \
         TUI,\n                                   \
         print the result (or write it to --output) and exit.\n                                   \
         Requires --dangerously-skip-permissions.\n  \
         --cwd <dir>                     Working directory for a headless --run (default: \
         current dir)\n  \
         --output <path>                 Write the --run result to this file instead of \
         stdout\n  \
         --api-key <key>                 Overlay an API key for the active provider for this \
         --run only\n                                   \
         (session-only, not persisted)\n  \
         -h, --help                      Show this help\n"
    );
}
