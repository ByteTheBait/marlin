pub mod context;
pub mod loop_guard;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::history::{
    self, InputHistory, Session, SessionEntry, from_session_message, to_session_message,
};
use crate::index::{self, Index};
use crate::providers::{
    Message, Provider, RateLimitState, StreamRequest, ToolCall, ToolCallMsg, registry::Registry,
};
use crate::snapshots;
use crate::tools::{all_tools, executor};
use context::{estimate_tokens, maybe_prune_history};
use loop_guard::LoopGuard;

// ── Channel types ────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum UiUpdate {
    StreamChunk(String),
    ToolCall { name: String, input: String },
    ToolResult { name: String, output: String, is_error: bool },
    SystemMsg(String),
    ErrorMsg(String),
    RateLimited { secs: u32 },
    GoalComplete { tool_count: usize },
    StatusUpdate(StatusInfo),
    IndexBuilt { files: usize, terms: usize },
}

#[derive(Debug, Clone)]
pub struct StatusInfo {
    pub provider: String,
    pub model: String,
}

#[derive(Debug)]
pub enum Action {
    SendMessage(String),
    SlashCommand(String),
    CancelStream,
    Quit,
}

// ── Engine ───────────────────────────────────────────────────────────────────

pub struct Engine {
    cfg: Config,
    registry: Registry,
    marlin_dir: PathBuf,
    work_dir: String,

    history: Vec<Message>,
    code_index: Option<Index>,
    session: Option<Session>,
    input_history: InputHistory,

    active_goal: String,
    tool_iterations: usize,
    attachments: Vec<(String, String)>, // (filename, content)
    allowed_commands: Vec<String>,

    rate_limit_state: Option<RateLimitState>,
    loop_guard: LoopGuard,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
}

impl Engine {
    pub fn new(cfg: Config) -> Result<Self> {
        let marlin_dir = crate::config::marlin_dir()?;
        let registry = Registry::new(&cfg);
        let work_dir = cfg.work_dir.clone();
        let allowed = cfg.allowed_commands.clone();

        let input_history = InputHistory::load(&marlin_dir);
        let code_index = index::load(&marlin_dir, &work_dir).ok();

        let project_name = Path::new(&work_dir).file_name()
            .unwrap_or_default().to_string_lossy().to_string();
        let session = Some(Session::new(&project_name, &work_dir));

        Ok(Self {
            cfg,
            registry,
            marlin_dir,
            work_dir,
            history: vec![],
            code_index,
            session,
            input_history,
            active_goal: String::new(),
            tool_iterations: 0,
            attachments: vec![],
            allowed_commands: allowed,
            rate_limit_state: None,
            loop_guard: LoopGuard::new(),
            cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    pub async fn run(
        &mut self,
        mut action_rx: mpsc::Receiver<Action>,
        ui_tx: mpsc::Sender<UiUpdate>,
    ) {
        // Send initial status
        let _ = ui_tx.send(UiUpdate::StatusUpdate(StatusInfo {
            provider: self.cfg.active_provider.clone(),
            model: self.cfg.active_model.clone(),
        })).await;

        let _ = ui_tx.send(UiUpdate::SystemMsg("marlin ready  /help for commands".into())).await;
        if let Some(idx) = &self.code_index {
            let _ = ui_tx.send(UiUpdate::SystemMsg(
                format!("index: {} files, {} terms", idx.file_count, idx.term_count)
            )).await;
        }

        while let Some(action) = action_rx.recv().await {
            match action {
                Action::Quit => break,

                Action::CancelStream => {
                    self.cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    self.active_goal.clear();
                    self.tool_iterations = 0;
                    let _ = ui_tx.send(UiUpdate::SystemMsg("Cancelled.".into())).await;
                }

                Action::SendMessage(text) => {
                    self.input_history.add(&text);
                    let content = self.build_message_content(&text);
                    self.history.push(Message {
                        role: "user".into(),
                        content,
                        tool_calls: vec![],
                        tool_use_id: String::new(),
                        tool_call_id: String::new(),
                        is_error: false,
                    });
                    self.attachments.clear();
                    self.active_goal = text;
                    self.tool_iterations = 0;
                    self.loop_guard.reset();
                    self.cancel_flag.store(false, std::sync::atomic::Ordering::SeqCst);
                    self.agentic_loop(&ui_tx).await;
                }

                Action::SlashCommand(cmd) => {
                    self.input_history.add(&cmd);
                    self.handle_slash_command(&cmd, &ui_tx).await;
                }
            }
        }
    }

    async fn agentic_loop(&mut self, ui_tx: &mpsc::Sender<UiUpdate>) {
        const SAFETY_CAP: usize = 100;

        loop {
            if self.cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }

            // Proactive rate-limit check
            if let Some(rl) = &self.rate_limit_state {
                let est = estimate_tokens(&self.history, &self.effective_system_prompt());
                let mut wait_secs = 0u32;

                if rl.remaining_tokens >= 0 && est as i64 > rl.remaining_tokens {
                    if let Some(reset) = rl.reset_tokens_at {
                        if let Ok(d) = reset.duration_since(SystemTime::now()) {
                            wait_secs = d.as_secs() as u32 + 1;
                        }
                    }
                }
                if rl.remaining_requests == 0 {
                    if let Some(reset) = rl.reset_requests_at {
                        if let Ok(d) = reset.duration_since(SystemTime::now()) {
                            let s = d.as_secs() as u32 + 1;
                            if s > wait_secs { wait_secs = s; }
                        }
                    }
                }

                if wait_secs > 0 {
                    self.rate_limit_state = None;
                    let _ = ui_tx.send(UiUpdate::RateLimited { secs: wait_secs }).await;
                    tokio::time::sleep(Duration::from_secs(wait_secs as u64)).await;
                    let _ = ui_tx.send(UiUpdate::SystemMsg("Rate limit cleared — resuming...".into())).await;
                }
            }

            let (compressed, dropped) = maybe_prune_history(&mut self.history);
            if compressed > 0 || dropped > 0 {
                let _ = ui_tx.send(UiUpdate::SystemMsg(format!(
                    "Context managed: compressed {compressed} messages, dropped {dropped} oldest turns."
                ))).await;
            }

            let provider = match self.registry.get(&self.cfg.active_provider) {
                Ok(p) => p,
                Err(e) => {
                    let _ = ui_tx.send(UiUpdate::ErrorMsg(e.to_string())).await;
                    break;
                }
            };

            let req = StreamRequest {
                model: self.cfg.active_model.clone(),
                messages: self.history.clone(),
                system_prompt: self.effective_system_prompt(),
                max_tokens: self.cfg.max_tokens,
                tools: all_tools(),
            };

            let mut stream = match provider.stream(req).await {
                Ok(s) => s,
                Err(e) => {
                    let _ = ui_tx.send(UiUpdate::ErrorMsg(e.to_string())).await;
                    break;
                }
            };

            let mut text_buf = String::new();
            let mut done_chunk = None;

            while let Some(chunk) = stream.recv().await {
                if self.cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
                    if !text_buf.is_empty() {
                        let _ = ui_tx.send(UiUpdate::StreamChunk("\n\n*[cancelled]*".into())).await;
                    }
                    return;
                }

                if chunk.retry_after > 0 {
                    let _ = ui_tx.send(UiUpdate::RateLimited { secs: chunk.retry_after }).await;
                    tokio::time::sleep(Duration::from_secs(chunk.retry_after as u64)).await;
                    let _ = ui_tx.send(UiUpdate::SystemMsg("Rate limit cleared — resuming...".into())).await;
                    // Retry from outer loop
                    break;
                }

                if let Some(e) = chunk.error {
                    let _ = ui_tx.send(UiUpdate::ErrorMsg(e.to_string())).await;
                    return;
                }

                if !chunk.content.is_empty() {
                    text_buf.push_str(&chunk.content);
                    let _ = ui_tx.send(UiUpdate::StreamChunk(chunk.content)).await;
                }

                if chunk.done {
                    if let Some(rl) = chunk.rate_limit {
                        self.rate_limit_state = Some(rl);
                    }
                    done_chunk = Some(chunk.tool_calls);
                    break;
                }
            }

            let Some(tool_calls) = done_chunk else { continue };

            if !tool_calls.is_empty() {
                if self.tool_iterations >= SAFETY_CAP {
                    let _ = ui_tx.send(UiUpdate::ErrorMsg(format!(
                        "Safety cap reached ({SAFETY_CAP} tool calls). Send a new message to continue."
                    ))).await;
                    self.active_goal.clear();
                    return;
                }

                let text = text_buf.trim().to_string();
                self.history.push(Message {
                    role: "assistant".into(),
                    content: text.clone(),
                    tool_calls: tool_calls.iter().map(|tc| ToolCallMsg {
                        id: tc.id.clone(), name: tc.name.clone(), input: tc.input.clone(),
                    }).collect(),
                    tool_use_id: String::new(),
                    tool_call_id: String::new(),
                    is_error: false,
                });

                // Notify TUI of each tool call
                for tc in &tool_calls {
                    let _ = ui_tx.send(UiUpdate::ToolCall {
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                    }).await;
                }

                // Execute tools (run in blocking thread)
                let results = self.execute_tools(&tool_calls).await;

                for (tc, res) in tool_calls.iter().zip(results.iter()) {
                    let _ = ui_tx.send(UiUpdate::ToolResult {
                        name: tc.name.clone(),
                        output: res.output.clone(),
                        is_error: res.is_error,
                    }).await;

                    // Loop guard check
                    if let Some(intercept) = self.loop_guard.check(&tc.name, res.is_error) {
                        let _ = ui_tx.send(UiUpdate::SystemMsg(intercept.clone())).await;
                        // Inject intercept as a tool result so the model sees it
                        self.history.push(Message {
                            role: "tool".into(),
                            content: intercept,
                            tool_calls: vec![],
                            tool_use_id: tc.id.clone(),
                            tool_call_id: tc.id.clone(),
                            is_error: true,
                        });
                        continue;
                    }

                    self.history.push(Message {
                        role: "tool".into(),
                        content: res.output.clone(),
                        tool_calls: vec![],
                        tool_use_id: tc.id.clone(),
                        tool_call_id: tc.id.clone(),
                        is_error: res.is_error,
                    });

                    // Keep index fresh after writes
                    if (tc.name == "write_file" || tc.name == "edit_file") && !res.is_error {
                        if let Some(path) = extract_file_path(&tc.input, &self.work_dir) {
                            if let Some(idx) = &mut self.code_index {
                                index::update_file(idx, &path);
                            }
                        }
                    }
                }

                self.tool_iterations += 1;
                // Continue loop
            } else {
                // Goal complete
                let text = text_buf.trim().to_string();
                if !text.is_empty() {
                    self.history.push(Message {
                        role: "assistant".into(),
                        content: text,
                        tool_calls: vec![],
                        tool_use_id: String::new(),
                        tool_call_id: String::new(),
                        is_error: false,
                    });
                } else if self.tool_iterations == 0 {
                    let _ = ui_tx.send(UiUpdate::ErrorMsg(
                        "Model returned an empty response. Try rephrasing or check your API key/quota.".into()
                    )).await;
                }

                let tool_count = self.tool_iterations;
                self.tool_iterations = 0;
                self.active_goal.clear();
                self.save_session();

                let _ = ui_tx.send(UiUpdate::GoalComplete { tool_count }).await;
                break;
            }
        }
    }

    async fn execute_tools(&self, calls: &[ToolCall]) -> Vec<executor::ToolResult> {
        let mut results = Vec::new();
        for call in calls {
            let name = call.name.clone();
            let input = call.input.clone();
            let work_dir = self.work_dir.clone();
            let allowed = self.allowed_commands.clone();
            let marlin_dir = self.marlin_dir.clone();
            let wd2 = work_dir.clone();

            let idx_clone = self.code_index.clone();

            let result = tokio::task::spawn_blocking(move || {
                let search_fn: Option<Box<dyn Fn(&str, usize) -> String>> =
                    idx_clone.map(|idx| {
                        let f: Box<dyn Fn(&str, usize) -> String> = Box::new(move |q: &str, lim: usize| {
                            let results = index::search(&idx, q, lim);
                            index::format_results(&results, q)
                        });
                        f
                    });
                executor::execute(
                    &name,
                    &input,
                    &work_dir,
                    &|cmd| allowed.iter().any(|p| p == "*" || cmd.starts_with(p.as_str())),
                    search_fn.as_deref(),
                    Some(&|abs_path: &str, tool: &str| {
                        snapshots::take(&marlin_dir, &wd2, abs_path, tool);
                    }),
                )
            }).await.unwrap_or_else(|e| executor::ToolResult {
                output: e.to_string(),
                is_error: true,
            });

            results.push(result);
        }
        results
    }

    fn effective_system_prompt(&self) -> String {
        let mut s = String::new();
        s.push_str("You are Marlin, an AI coding assistant running in a terminal.\n");
        s.push_str("You help the user write, debug, and understand code.\n\n");

        s.push_str("## Tools\n");
        s.push_str("You have the following tools and MUST use them to act directly — never tell the user to do something manually that a tool can do:\n");
        s.push_str("- read_file: read any file. Pass 'function' to extract just one named function — far cheaper for large codebases\n");
        s.push_str("- write_file: create or overwrite a file\n");
        s.push_str("- edit_file: replace a specific string in a file (preferred for targeted edits)\n");
        s.push_str("- run_command: run a shell command\n");
        s.push_str("- list_directory: list files in a directory\n");
        s.push_str("- create_directory: create a directory\n");
        if self.code_index.is_some() {
            let idx = self.code_index.as_ref().unwrap();
            s.push_str(&format!(
                "- search_codebase: search {} indexed files with TF-IDF — use this to find relevant files before reading them\n",
                idx.file_count
            ));
        }
        s.push('\n');
        s.push_str("When asked to create a file, write code, edit something, or run a command — DO IT with the appropriate tool. ");
        s.push_str("Do not explain how the user could do it themselves. Do not ask for confirmation before using tools. Just act.\n\n");

        s.push_str(&format!("Working directory: {}\n\n", self.work_dir));

        if !self.active_goal.is_empty() {
            s.push_str("## Active Goal\n");
            s.push_str(&self.active_goal);
            s.push('\n');
            s.push_str("\nWork toward this goal using tools. Keep calling tools until the task is fully complete.\n");
            s.push_str("Only produce a plain text response (with no tool calls) when the goal is achieved or you need user input.\n");
            s.push_str(&format!("Progress so far: {} tool calls made.\n", self.tool_iterations));
        }

        if !self.cfg.system_prompt.is_empty() {
            s.push_str("\nAdditional instructions:\n");
            s.push_str(&self.cfg.system_prompt);
        }

        s
    }

    fn build_message_content(&self, text: &str) -> String {
        if self.attachments.is_empty() { return text.to_string(); }
        let mut s = String::new();
        for (filename, content) in &self.attachments {
            let ext = Path::new(filename).extension()
                .and_then(|e| e.to_str()).unwrap_or("");
            s.push_str(&format!("File: {filename}\n```{ext}\n{content}\n```\n\n"));
        }
        s.push_str(text);
        s
    }

    fn save_session(&mut self) {
        let Some(session) = &mut self.session else { return };
        session.messages = self.history.iter().map(to_session_message).collect();
        history::save_session(&self.marlin_dir, session);
    }

    // ── Slash command handler ─────────────────────────────────────────────────

    async fn handle_slash_command(&mut self, raw: &str, ui_tx: &mpsc::Sender<UiUpdate>) {
        let parts: Vec<&str> = raw.trim().splitn(2, ' ').collect();
        let cmd = parts[0].to_lowercase();
        let args: Vec<&str> = if parts.len() > 1 {
            parts[1].split_whitespace().collect()
        } else {
            vec![]
        };
        let rest = parts.get(1).copied().unwrap_or("").trim();

        macro_rules! sys {
            ($msg:expr) => {{ ui_tx.send(UiUpdate::SystemMsg($msg.into())).await.ok(); }};
        }
        macro_rules! err {
            ($msg:expr) => {{ ui_tx.send(UiUpdate::ErrorMsg($msg.into())).await.ok(); }};
        }

        match cmd.as_str() {
            "/help" => {
                sys!(help_text());
            }

            "/clear" => {
                self.history.clear();
                self.attachments.clear();
                sys!("Chat cleared.");
            }

            "/provider" | "/p" => {
                if args.is_empty() {
                    sys!(format!("Usage: /provider <name>  — available: {}", self.registry.names().join(", ")));
                    return;
                }
                let name = args[0].to_lowercase();
                if self.registry.get(&name).is_err() {
                    err!(format!("Unknown provider: {name}"));
                    return;
                }
                self.cfg.active_provider = name.clone();
                let model = self.cfg.providers.get(&name)
                    .and_then(|p| if p.model.is_empty() { None } else { Some(p.model.clone()) })
                    .unwrap_or_default();
                self.cfg.active_model = model.clone();
                let _ = self.cfg.save();
                sys!(format!("Switched to provider: {name}  model: {model}"));
                let _ = ui_tx.send(UiUpdate::StatusUpdate(StatusInfo {
                    provider: name, model,
                })).await;
            }

            "/model" | "/m" => {
                if args.is_empty() {
                    if let Ok(p) = self.registry.get(&self.cfg.active_provider) {
                        sys!(format!("Available models: {}", p.models().join(", ")));
                    }
                    return;
                }
                let model = args[0].to_string();
                self.cfg.active_model = model.clone();
                if let Some(pcfg) = self.cfg.providers.get_mut(&self.cfg.active_provider) {
                    pcfg.model = model.clone();
                }
                let _ = self.cfg.save();
                sys!(format!("Model set to: {model}"));
                let _ = ui_tx.send(UiUpdate::StatusUpdate(StatusInfo {
                    provider: self.cfg.active_provider.clone(),
                    model,
                })).await;
            }

            "/key" => {
                if args.is_empty() {
                    sys!("Usage: /key <provider> <api-key>");
                    return;
                }
                if args.len() < 2 {
                    sys!("Usage: /key <provider> <api-key>");
                    return;
                }
                let provider = args[0].to_lowercase();
                let key = args[1];
                self.cfg.set_key(&provider, key);
                let _ = self.cfg.save();
                self.registry = Registry::new(&self.cfg);
                sys!(format!("API key saved for {provider}."));
            }

            "/endpoint" => {
                if args.len() < 2 {
                    sys!("Usage: /endpoint <provider> <url>");
                    return;
                }
                self.cfg.set_endpoint(args[0], args[1]);
                let _ = self.cfg.save();
                self.registry = Registry::new(&self.cfg);
                sys!(format!("Endpoint updated for {}: {}", args[0], args[1]));
            }

            "/system" | "/sys" => {
                if rest.is_empty() {
                    if self.cfg.system_prompt.is_empty() {
                        sys!("No custom system prompt. Use /system <text> to set one.");
                    } else {
                        sys!(format!("Custom system prompt: {}", self.cfg.system_prompt));
                    }
                    return;
                }
                self.cfg.system_prompt = rest.to_string();
                let _ = self.cfg.save();
                sys!("System prompt updated.");
            }

            "/tokens" => {
                if args.is_empty() {
                    sys!(format!("Max tokens: {}  (use /tokens <n> to change)", self.cfg.max_tokens));
                    return;
                }
                if let Ok(n) = args[0].parse::<usize>() {
                    if n > 0 {
                        self.cfg.max_tokens = n;
                        let _ = self.cfg.save();
                        sys!(format!("Max tokens: {n}"));
                    }
                }
            }

            "/providers" => {
                let names = self.registry.names();
                let mut lines: Vec<String> = Vec::new();
                for n in &names {
                    let mark = if n == &self.cfg.active_provider { "▶ " } else { "  " };
                    if let Ok(p) = self.registry.get(n) {
                        let models = p.models();
                        let preview: Vec<String> = models.iter().take(2).cloned().collect();
                        lines.push(format!("{mark}{n}  [{}...]", preview.join(", ")));
                    }
                }
                sys!(format!("Providers:\n{}", lines.join("\n")));
            }

            "/models" => {
                match self.registry.get(&self.cfg.active_provider) {
                    Ok(p) => sys!(format!("Models for {}:\n{}", self.cfg.active_provider, p.models().join("\n"))),
                    Err(e) => err!(e.to_string()),
                }
            }

            "/attach" | "/a" => {
                if args.is_empty() {
                    if self.attachments.is_empty() {
                        sys!("No files attached. Usage: /attach <file>");
                    } else {
                        let names: Vec<String> = self.attachments.iter()
                            .map(|(f, c)| format!("{} ({} lines)", f, c.lines().count()))
                            .collect();
                        sys!(format!("Attached:\n{}", names.join("\n")));
                    }
                    return;
                }
                let path = self.resolve_path(args[0]);
                match std::fs::read_to_string(&path) {
                    Ok(content) => {
                        let lines = content.lines().count();
                        self.attachments.retain(|(f, _)| f != &path);
                        self.attachments.push((path.clone(), content));
                        sys!(format!("Attached: {} ({lines} lines) — send your next message to include it",
                            Path::new(&path).file_name().unwrap_or_default().to_string_lossy()));
                    }
                    Err(e) => err!(format!("attach error: {e}")),
                }
            }

            "/detach" => {
                if args.is_empty() {
                    self.attachments.clear();
                    sys!("All attachments cleared.");
                } else {
                    let name = args[0];
                    let before = self.attachments.len();
                    self.attachments.retain(|(f, _)| {
                        Path::new(f).file_name().and_then(|n| n.to_str()) != Some(name) && f != name
                    });
                    if self.attachments.len() < before {
                        sys!(format!("Detached: {name}"));
                    } else {
                        err!(format!("No attachment named {name:?}"));
                    }
                }
            }

            "/exec" => {
                if rest.is_empty() {
                    sys!("Usage: /exec <shell command>");
                    return;
                }
                if !self.is_allowed(rest) {
                    let first = rest.split_whitespace().next().unwrap_or(rest);
                    err!(format!("Command not allowed: {rest:?}\nUse /allow {first} to permit it."));
                    return;
                }
                sys!(format!("Running: {rest}"));
                let cmd = rest.to_string();
                let wd = self.work_dir.clone();
                let out = tokio::task::spawn_blocking(move || {
                    std::process::Command::new("sh").arg("-c").arg(&cmd).current_dir(&wd).output()
                }).await;
                match out {
                    Ok(Ok(o)) => {
                        let text = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
                        sys!(format!("[exec]\n{}", text.trim()));
                    }
                    _ => err!("exec failed"),
                }
            }

            "/allow" => {
                if args.is_empty() {
                    if self.allowed_commands.is_empty() {
                        sys!("No commands allowed. Use /allow <prefix> to permit.");
                    } else {
                        sys!(format!("Allowed prefixes: {}", self.allowed_commands.join(", ")));
                    }
                    return;
                }
                let pattern = rest.to_string();
                self.allowed_commands.push(pattern.clone());
                self.cfg.allowed_commands = self.allowed_commands.clone();
                let _ = self.cfg.save();
                sys!(format!("Allowed: {pattern:?}"));
            }

            "/index" => {
                if args.first().copied() == Some("status") {
                    if let Some(idx) = &self.code_index {
                        sys!(format!("Index: {} files, {} terms, built {}.",
                            idx.file_count, idx.term_count,
                            idx.built_at.format("%b %d %H:%M")));
                    } else {
                        sys!("No index built. Run /index to build one.");
                    }
                    return;
                }
                let wd = self.work_dir.clone();
                sys!(format!("Building index for {wd}…"));
                let result = tokio::task::spawn_blocking(move || index::build(&wd, None)).await;
                match result {
                    Ok(Ok((idx, stats))) => {
                        let _ = ui_tx.send(UiUpdate::IndexBuilt {
                            files: stats.files,
                            terms: stats.terms,
                        }).await;
                        index::save(&self.marlin_dir, &idx);
                        sys!(format!("Index built: {} files, {} terms in {:?}. Use /search <query> or the AI will use it automatically.",
                            stats.files, stats.terms, stats.elapsed));
                        self.code_index = Some(idx);
                    }
                    _ => err!("Index build failed"),
                }
            }

            "/search" => {
                if rest.is_empty() {
                    sys!("Usage: /search <query>");
                    return;
                }
                let Some(idx) = &self.code_index else {
                    err!("No index. Run /index first.");
                    return;
                };
                let results = index::search(idx, rest, 8);
                sys!(index::format_results(&results, rest));
            }

            "/revert" => {
                if args.is_empty() {
                    sys!("Usage: /revert <file> [n]  —  list snapshots or restore one");
                    return;
                }
                let abs_path = self.resolve_path(args[0]);
                let snaps = snapshots::list(&self.marlin_dir, &self.work_dir, &abs_path);
                if snaps.is_empty() {
                    sys!(format!("No snapshots for {} — Marlin snapshots files before every AI edit.", args[0]));
                    return;
                }
                if args.len() < 2 {
                    let lines: Vec<String> = snaps.iter().enumerate().map(|(i, s)| {
                        format!("  {:2}.  {}  [{}]  {}",
                            i + 1,
                            s.timestamp.format("%b %d %H:%M:%S"),
                            s.tool,
                            snapshots::human_size(s.size))
                    }).collect();
                    sys!(format!("Snapshots for {} (newest first):\n{}\n\nUse /revert {} <n> to restore.",
                        args[0], lines.join("\n"), args[0]));
                    return;
                }
                let n: usize = args[1].parse().unwrap_or(0);
                if n < 1 || n > snaps.len() {
                    err!(format!("Invalid snapshot number (1–{}).", snaps.len()));
                    return;
                }
                let snap = &snaps[n - 1];
                match snapshots::restore(&self.marlin_dir, &self.work_dir, &abs_path, &snap.id) {
                    Ok(_) => sys!(format!("Restored {} → snapshot from {} ({}, {}).",
                        args[0], snap.timestamp.format("%b %d %H:%M:%S"), snap.tool, snapshots::human_size(snap.size))),
                    Err(e) => err!(format!("Restore failed: {e}")),
                }
            }

            "/resume" => {
                match history::list_sessions(&self.marlin_dir) {
                    Ok(sessions) if !sessions.is_empty() => {
                        let s = &sessions[0];
                        self.history = s.messages.iter().map(from_session_message).collect();
                        sys!(format!("Resumed: {}", s.summary()));
                    }
                    _ => sys!("No saved sessions to resume."),
                }
            }

            "/history" => {
                if args.first().copied() == Some("clear") {
                    match history::clear_sessions(&self.marlin_dir) {
                        Ok(_) => sys!("Session history cleared."),
                        Err(e) => err!(format!("Failed to clear sessions: {e}")),
                    }
                    return;
                }
                let sessions = history::list_sessions(&self.marlin_dir).unwrap_or_default();
                if sessions.is_empty() {
                    sys!("No saved sessions.");
                    return;
                }
                if let Some(n_str) = args.first() {
                    if let Ok(n) = n_str.parse::<usize>() {
                        if n >= 1 && n <= sessions.len() {
                            let s = &sessions[n - 1];
                            self.history = s.messages.iter().map(from_session_message).collect();
                            sys!(format!("Loaded: {}", s.summary()));
                            return;
                        }
                        err!(format!("Invalid session number (1–{}).", sessions.len()));
                        return;
                    }
                }
                let limit = sessions.len().min(20);
                let lines: Vec<String> = sessions[..limit].iter().enumerate()
                    .map(|(i, s)| format!("  {:2}.  {}  [{}]", i + 1, s.summary(), s.project))
                    .collect();
                sys!(format!("Saved sessions (newest first):\n{}\n\nUse /history <n> to load, /history clear to delete all.",
                    lines.join("\n")));
            }

            "/cat" => {
                if args.is_empty() { sys!("Usage: /cat <file>"); return; }
                let path = self.resolve_path(args[0]);
                match std::fs::read_to_string(&path) {
                    Ok(content) => sys!(format!("[{path}]\n{content}")),
                    Err(e) => err!(e.to_string()),
                }
            }

            "/ls" => {
                let dir = if args.is_empty() {
                    self.work_dir.clone()
                } else {
                    self.resolve_path(args[0])
                };
                match std::fs::read_dir(&dir) {
                    Err(e) => err!(e.to_string()),
                    Ok(entries) => {
                        let mut names: Vec<String> = entries.filter_map(|e| e.ok())
                            .map(|e| {
                                let n = e.file_name().to_string_lossy().to_string();
                                if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { n + "/" } else { n }
                            }).collect();
                        names.sort();
                        sys!(format!("[{dir}]\n{}", names.join("\n")));
                    }
                }
            }

            "/cd" => {
                if args.is_empty() {
                    sys!(format!("Current directory: {}", self.work_dir));
                    return;
                }
                let new_dir = self.resolve_path(args[0]);
                match std::fs::metadata(&new_dir) {
                    Ok(m) if m.is_dir() => {
                        self.work_dir = new_dir.clone();
                        self.cfg.work_dir = new_dir.clone();
                        let _ = self.cfg.save();
                        sys!(format!("Directory: {new_dir}"));
                    }
                    _ => err!(format!("Not a directory: {}", args[0])),
                }
            }

            "/pwd" => {
                sys!(format!("Directory: {}", self.work_dir));
            }

            _ => {
                err!(format!("Unknown command: {cmd}  (type /help for list)"));
            }
        }
    }

    fn is_allowed(&self, cmd: &str) -> bool {
        self.allowed_commands.iter().any(|p| p == "*" || cmd.starts_with(p.as_str()))
    }

    fn resolve_path(&self, p: &str) -> String {
        if p == "~" {
            return dirs::home_dir().unwrap_or_default().to_string_lossy().to_string();
        }
        if let Some(rest) = p.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest).to_string_lossy().to_string();
            }
        }
        if Path::new(p).is_absolute() { return p.to_string(); }
        format!("{}/{}", self.work_dir, p)
    }
}

fn extract_file_path(input_json: &str, work_dir: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(input_json).ok()?;
    let p = v["path"].as_str()?;
    if p.is_empty() { return None; }
    if Path::new(p).is_absolute() {
        Some(p.to_string())
    } else {
        Some(format!("{work_dir}/{p}"))
    }
}

fn help_text() -> String {
    let cmds = [
        ("/help", "show this help"),
        ("/clear", "clear chat history and attachments"),
        ("/provider <name>", "switch provider (claude/ollama/groq/fireworks/moonshot/custom)"),
        ("/model <name>", "switch model"),
        ("/providers", "list all providers and models"),
        ("/models", "list models for current provider"),
        ("/key <provider> <key>", "set API key"),
        ("/endpoint <provider> <url>", "set API endpoint for a provider"),
        ("/system <prompt>", "set additional system prompt"),
        ("/tokens <n>", "set max output tokens"),
        ("/attach <file>", "attach a file to your next message"),
        ("/detach [file]", "remove attachment(s)"),
        ("/exec <cmd>", "run a shell command (must be /allow-ed first)"),
        ("/allow <prefix>", "allow a shell command prefix (e.g. /allow npm)"),
        ("/index [status]", "build (or check) the TF-IDF codebase search index"),
        ("/search <query>", "search the index and show ranked results with snippets"),
        ("/revert <file> [n]", "list file snapshots or restore one"),
        ("/resume", "resume the most recent saved session"),
        ("/history [n|clear]", "list saved sessions, load one by number, or clear all"),
        ("/cat <file>", "print file contents"),
        ("/ls [dir]", "list directory"),
        ("/cd <dir>", "change working directory"),
        ("/pwd", "show working directory"),
    ];

    let mut s = "Commands:\n".to_string();
    for (cmd, desc) in &cmds {
        let pad = 32usize.saturating_sub(cmd.len());
        s.push_str(&format!("  {}{}{}\n", cmd, " ".repeat(pad.max(1)), desc));
    }
    s
}
