pub mod context;
pub mod loop_guard;
pub mod tasks;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use tokio::sync::mpsc;

use crate::config::{AstMode, Config, ModelTier, SandboxMode};
use crate::history::{
    self, InputHistory, Session, from_session_message, to_session_message,
};
use crate::index::{self, Index};
use crate::providers::{
    Message, RateLimitState, StreamRequest, ToolCall, ToolCallMsg, registry::Registry,
};
use crate::skills::{self, Skill, SkillDef};
use crate::snapshots;
use crate::tools::{all_tools, executor, policy};
use context::{estimate_tokens, maybe_prune_history};
use loop_guard::LoopGuard;
use tasks::{TaskStatus, TaskStep};

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
    IndexBuilt,
    /// Engine is paused waiting for user approval of a destructive command
    AwaitingApproval { cmd: String },
    /// Updated task list for the sidebar
    TaskUpdate(Vec<TaskStep>),
    /// Token budget update for the sidebar meter
    TokenUsage { used: usize, budget: usize },
    /// AST mode changed — drives the status bar badge
    AstMode(AstMode),
    /// Skills loaded on startup — TUI uses these for typing suggestions.
    SkillsLoaded(Vec<SkillDef>),
    /// Skill keyword matches for the most recent user message.
    SkillMatches(Vec<(String, String)>),
    /// Difficulty score and selected tier for the current request.
    TierSelected { score: u8, tier: String },
}

#[derive(Debug, Clone)]
pub struct StatusInfo {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone)]
pub enum Action {
    SendMessage(String),
    SlashCommand(String),
    CancelStream,
    /// User approved a destructive command in the modal
    Approve,
    /// User denied a destructive command in the modal
    Deny,
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

    /// Live task list for the sidebar
    task_steps: Vec<TaskStep>,
    /// Approximate token budget ceiling (from config or 100k default)
    token_budget: usize,
    /// AST-driven context mode
    ast_mode: AstMode,

    /// Loaded skill definitions
    skills: Vec<Skill>,
    /// Provider/model selected for the current agentic request (may be tier-routed)
    req_provider: String,
    req_model: String,
    /// Backup provider/model to use if req_provider is rate-limited
    req_backup_provider: String,
    req_backup_model: String,

    /// Token count at last LLM compaction — prevents immediate re-triggering.
    compact_guard_tokens: usize,
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

        let ast_mode = cfg.ast_mode.clone();
        let req_provider = cfg.active_provider.clone();
        let req_model = cfg.active_model.clone();

        // Install built-in skills if not present, then load all.
        skills::install_defaults(&marlin_dir);
        let loaded_skills = skills::load_all(&marlin_dir);

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
            task_steps: vec![],
            token_budget: 100_000,
            ast_mode,
            skills: loaded_skills,
            req_provider,
            req_model,
            req_backup_provider: String::new(),
            req_backup_model: String::new(),
            compact_guard_tokens: 0,
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
        if self.ast_mode != AstMode::Off {
            let _ = ui_tx.send(UiUpdate::AstMode(self.ast_mode.clone())).await;
        }
        if let Some(idx) = &self.code_index {
            let _ = ui_tx.send(UiUpdate::SystemMsg(
                format!("index: {} files, {} terms", idx.file_count, idx.term_count)
            )).await;
        }

        // Send skills to TUI for suggestion panel.
        let skill_defs: Vec<SkillDef> = self.skills.iter().map(SkillDef::from).collect();
        let _ = ui_tx.send(UiUpdate::SkillsLoaded(skill_defs)).await;

        // Spawn nightly skill-suggestion daemon.
        self.maybe_spawn_daemon(ui_tx.clone());

        while let Some(action) = action_rx.recv().await {
            match action {
                Action::Quit => break,

                Action::CancelStream => {
                    self.cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                    self.active_goal.clear();
                    self.tool_iterations = 0;
                    let _ = ui_tx.send(UiUpdate::SystemMsg("Cancelled.".into())).await;
                }

                // Approval actions received outside an agentic loop are no-ops
                Action::Approve | Action::Deny => {}

                Action::SendMessage(text) => {
                    self.input_history.add(&text);

                    // Emit skill matches so TUI can show relevant skills.
                    let skill_defs: Vec<SkillDef> = self.skills.iter().map(SkillDef::from).collect();
                    let matches: Vec<(String, String)> = skills::suggest::match_skills(&text, &skill_defs)
                        .into_iter()
                        .map(|m| (m.name, m.description))
                        .collect();
                    if !matches.is_empty() {
                        let _ = ui_tx.send(UiUpdate::SkillMatches(matches)).await;
                    }

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
                    self.active_goal = text.clone();
                    self.tool_iterations = 0;
                    self.task_steps.clear();
                    self.loop_guard.reset();
                    self.cancel_flag.store(false, std::sync::atomic::Ordering::SeqCst);

                    // Select model tier based on difficulty score (if tiers enabled).
                    self.rate_and_route(&text, &ui_tx).await;

                    // Broadcast token count immediately so the sidebar isn't stale while waiting.
                    // Prefer exact count from provider API; fall back to heuristic.
                    let system_prompt = self.effective_system_prompt();
                    let tok = if let Ok(p) = self.registry.get(&self.req_provider) {
                        let req_for_count = StreamRequest {
                            model: self.req_model.clone(),
                            messages: self.history.clone(),
                            system_prompt: system_prompt.clone(),
                            max_tokens: 1,
                            tools: all_tools(&self.ast_mode, &self.skills.iter().map(|s| (s.name.clone(), s.description.clone())).collect::<Vec<_>>()),
                        };
                        p.count_tokens(&req_for_count).await
                            .unwrap_or_else(|| estimate_tokens(&self.history, &system_prompt))
                    } else {
                        estimate_tokens(&self.history, &system_prompt)
                    };
                    let _ = ui_tx.send(UiUpdate::TokenUsage {
                        used: tok,
                        budget: self.token_budget,
                    }).await;
                    self.agentic_loop(&ui_tx, &mut action_rx).await;
                }

                Action::SlashCommand(cmd) => {
                    self.input_history.add(&cmd);
                    self.handle_slash_command(&cmd, &ui_tx).await;
                }
            }
        }
    }

    async fn agentic_loop(&mut self, ui_tx: &mpsc::Sender<UiUpdate>, action_rx: &mut mpsc::Receiver<Action>) {
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

            // LLM-based compaction first, then mechanical truncation fallback
            self.maybe_compact_history(ui_tx).await;

            let (compressed, dropped) = maybe_prune_history(&mut self.history);
            if compressed > 0 || dropped > 0 {
                let _ = ui_tx.send(UiUpdate::SystemMsg(format!(
                    "Context managed: compressed {compressed} messages, dropped {dropped} oldest turns."
                ))).await;
            }

            // Broadcast token usage to sidebar
            let tok_used = estimate_tokens(&self.history, &self.effective_system_prompt());
            let _ = ui_tx.send(UiUpdate::TokenUsage {
                used: tok_used,
                budget: self.token_budget,
            }).await;

            let provider = match self.registry.get(&self.req_provider) {
                Ok(p) => p,
                Err(e) => {
                    let _ = ui_tx.send(UiUpdate::ErrorMsg(e.to_string())).await;
                    break;
                }
            };

            let req = StreamRequest {
                model: self.req_model.clone(),
                messages: self.history.clone(),
                system_prompt: self.effective_system_prompt(),
                max_tokens: self.cfg.max_tokens,
                tools: all_tools(&self.ast_mode, &self.skills.iter().map(|s| (s.name.clone(), s.description.clone())).collect::<Vec<_>>()),
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

            // Poll every 50 ms so Ctrl+C is felt within one frame rather than
            // waiting for the next network chunk (which can take seconds).
            'recv: loop {
                let chunk = tokio::select! {
                    maybe = stream.recv() => match maybe {
                        Some(c) => c,
                        None => break 'recv,
                    },
                    _ = tokio::time::sleep(Duration::from_millis(50)) => {
                        if self.cancel_flag.load(std::sync::atomic::Ordering::SeqCst) {
                            if !text_buf.is_empty() {
                                let _ = ui_tx.send(UiUpdate::StreamChunk("\n\n*[cancelled]*".into())).await;
                            }
                            return;
                        }
                        continue 'recv;
                    }
                };

                if chunk.retry_after > 0 {
                    // Switch to backup provider/model if configured.
                    if !self.req_backup_provider.is_empty() {
                        let bp = std::mem::take(&mut self.req_backup_provider);
                        let bm = std::mem::take(&mut self.req_backup_model);
                        let _ = ui_tx.send(UiUpdate::SystemMsg(format!(
                            "Rate limited — switching to backup: {bp} / {bm}"
                        ))).await;
                        self.req_provider = bp;
                        self.req_model = bm;
                    } else {
                        let _ = ui_tx.send(UiUpdate::RateLimited { secs: chunk.retry_after }).await;
                        tokio::time::sleep(Duration::from_secs(chunk.retry_after as u64)).await;
                        let _ = ui_tx.send(UiUpdate::SystemMsg("Rate limit cleared — resuming...".into())).await;
                    }
                    break 'recv;
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
                    break 'recv;
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

                // Notify TUI of each tool call and add to task list
                for tc in &tool_calls {
                    let _ = ui_tx.send(UiUpdate::ToolCall {
                        name: tc.name.clone(),
                        input: tc.input.clone(),
                    }).await;
                    let desc = tool_short_desc(&tc.name, &tc.input);
                    self.task_steps.push(TaskStep::tool_pending(&tc.name, desc));
                }
                let _ = ui_tx.send(UiUpdate::TaskUpdate(self.task_steps.clone())).await;

                // Check for destructive commands and await user approval
                let denied = self.run_approval_checks(&tool_calls, ui_tx, action_rx).await;
                if self.cancel_flag.load(std::sync::atomic::Ordering::SeqCst) { return; }

                // Execute tools (run in blocking thread)
                let results = self.execute_tools(&tool_calls, &denied).await;

                // Track which task step index corresponds to this batch
                let batch_task_start = self.task_steps.len().saturating_sub(tool_calls.len());

                for (i, (tc, res)) in tool_calls.iter().zip(results.iter()).enumerate() {
                    let _ = ui_tx.send(UiUpdate::ToolResult {
                        name: tc.name.clone(),
                        output: res.output.clone(),
                        is_error: res.is_error,
                    }).await;

                    // Update task step status
                    let step_idx = batch_task_start + i;
                    if step_idx < self.task_steps.len() {
                        self.task_steps[step_idx].status = if res.is_error {
                            TaskStatus::Failed
                        } else {
                            TaskStatus::Completed
                        };
                    }

                    // File-hash-aware loop guard for edits
                    let intercept = if tc.name == "edit_file" || tc.name == "write_file" {
                        if let Some(path) = extract_file_path(&tc.input, &self.work_dir) {
                            let content = std::fs::read(&path).unwrap_or_default();
                            self.loop_guard.check_file_edit(&path, &content, res.is_error)
                        } else {
                            self.loop_guard.check(&tc.name, res.is_error)
                        }
                    } else {
                        self.loop_guard.check(&tc.name, res.is_error)
                    };

                    if let Some(msg) = intercept {
                        let _ = ui_tx.send(UiUpdate::SystemMsg(msg.clone())).await;
                        self.history.push(Message {
                            role: "tool".into(),
                            content: msg,
                            tool_calls: vec![],
                            tool_use_id: tc.id.clone(),
                            tool_call_id: tc.id.clone(),
                            is_error: true,
                        });
                    } else {
                        self.history.push(Message {
                            role: "tool".into(),
                            content: res.output.clone(),
                            tool_calls: vec![],
                            tool_use_id: tc.id.clone(),
                            tool_call_id: tc.id.clone(),
                            is_error: res.is_error,
                        });
                    }

                    // Keep index fresh after writes
                    if (tc.name == "write_file" || tc.name == "edit_file") && !res.is_error {
                        if let Some(path) = extract_file_path(&tc.input, &self.work_dir) {
                            if let Some(idx) = &mut self.code_index {
                                index::update_file(idx, &path);
                            }
                        }
                    }
                }

                let _ = ui_tx.send(UiUpdate::TaskUpdate(self.task_steps.clone())).await;

                // Write-Test-Fix: run verify_command after any file edit
                let had_file_edit = tool_calls.iter().zip(results.iter())
                    .any(|(tc, r)| (tc.name == "edit_file" || tc.name == "write_file") && !r.is_error);
                if had_file_edit {
                    if let Some(verify_result) = self.run_verify_command(ui_tx).await {
                        self.history.push(verify_result);
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

    async fn execute_tools(&self, calls: &[ToolCall], denied: &HashSet<String>) -> Vec<executor::ToolResult> {
        let mut results = Vec::new();
        for call in calls {
            if denied.contains(&call.id) {
                results.push(executor::ToolResult {
                    output: "Command denied by user.".to_string(),
                    is_error: true,
                });
                continue;
            }

            // Handle skill invocations directly — needs access to self.skills
            if call.name == "run_skill" {
                let input_map: std::collections::HashMap<String, String> =
                    serde_json::from_str(&call.input).unwrap_or_default();
                let skill_name = input_map.get("name").cloned().unwrap_or_default();
                let query = input_map.get("query").cloned().unwrap_or_default();

                let result = if let Some(skill) = self.skills.iter().find(|s| s.name == skill_name) {
                    match skill.run.kind {
                        skills::SkillKind::Shell => {
                            match skills::executor::skill_command(skill, &query) {
                                Err(e) => executor::ToolResult { output: e.to_string(), is_error: true },
                                Ok(cmd) => {
                                    let cmd_json = serde_json::json!({"command": cmd}).to_string();
                                    let wd = self.work_dir.clone();
                                    let clean_env = self.cfg.clean_env;
                                    let logs_dir = self.marlin_dir.join("logs");
                                    tokio::task::spawn_blocking(move || {
                                        executor::execute(
                                            "run_command",
                                            &cmd_json,
                                            &wd,
                                            &|_| true,  // skills bypass the command allow-list
                                            None,
                                            None,
                                            Some(&logs_dir),
                                            clean_env,
                                            crate::config::AstMode::Off,
                                            &crate::config::SandboxMode::Off, // skills run outside sandbox
                                        )
                                    }).await.unwrap_or_else(|e| executor::ToolResult {
                                        output: e.to_string(),
                                        is_error: true,
                                    })
                                }
                            }
                        }
                        skills::SkillKind::Prompt => {
                            match skills::executor::expand_prompt(skill, &query) {
                                Ok(expanded) => executor::ToolResult { output: expanded, is_error: false },
                                Err(e) => executor::ToolResult { output: e.to_string(), is_error: true },
                            }
                        }
                    }
                } else {
                    executor::ToolResult { output: format!("skill '{skill_name}' not found"), is_error: true }
                };
                results.push(result);
                continue;
            }

            let name = call.name.clone();
            let input = call.input.clone();
            let work_dir = self.work_dir.clone();
            let allowed = self.allowed_commands.clone();
            let marlin_dir = self.marlin_dir.clone();
            let wd2 = work_dir.clone();
            let logs_dir = marlin_dir.join("logs");
            let sandbox = self.cfg.sandbox_mode.allows_all() || self.cfg.skip_permissions;
            let clean_env = self.cfg.clean_env;
            let ast_mode = self.ast_mode.clone();
            let sandbox_mode = self.cfg.sandbox_mode.clone();

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
                    &|cmd| sandbox || policy::is_command_allowed(cmd, &allowed),
                    search_fn.as_deref(),
                    Some(&|abs_path: &str, tool: &str| {
                        snapshots::take(&marlin_dir, &wd2, abs_path, tool);
                    }),
                    Some(&logs_dir),
                    clean_env,
                    ast_mode,
                    &sandbox_mode,
                )
            }).await.unwrap_or_else(|e| executor::ToolResult {
                output: e.to_string(),
                is_error: true,
            });

            results.push(result);
        }
        results
    }

    /// Returns the set of tool call IDs the user denied.
    async fn run_approval_checks(
        &mut self,
        calls: &[ToolCall],
        ui_tx: &mpsc::Sender<UiUpdate>,
        action_rx: &mut mpsc::Receiver<Action>,
    ) -> HashSet<String> {
        let mut denied = HashSet::new();
        for tc in calls {
            if tc.name != "run_command" { continue; }
            let cmd = extract_cmd_str(&tc.input);
            if !is_destructive_cmd(&cmd) { continue; }

            let _ = ui_tx.send(UiUpdate::AwaitingApproval { cmd }).await;

            // Wait for approval response, ignoring unrelated actions
            let approved = loop {
                match action_rx.recv().await {
                    Some(Action::Approve) => break true,
                    Some(Action::Deny)    => break false,
                    Some(Action::CancelStream) => {
                        self.cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                        break false;
                    }
                    None => break false,
                    _ => {} // ignore other actions while modal is open
                }
            };
            if !approved {
                denied.insert(tc.id.clone());
            }
        }
        denied
    }

    /// Run the configured verify_command after a file edit. Returns a Message to inject if
    /// the command fails (or None if passing / not configured).
    async fn run_verify_command(&self, ui_tx: &mpsc::Sender<UiUpdate>) -> Option<Message> {
        let cmd = self.cfg.verify_command.as_deref()?.to_string();
        let work_dir = self.work_dir.clone();
        let clean_env = self.cfg.clean_env;

        let _ = ui_tx.send(UiUpdate::SystemMsg(format!("[Marlin Verify] Running: {cmd}"))).await;

        let result = match tokio::task::spawn_blocking(move || {
            let mut command = std::process::Command::new("sh");
            command.arg("-c").arg(&cmd).current_dir(&work_dir);
            if clean_env {
                command.env_clear();
                for var in &["PATH", "HOME", "USER", "LANG", "CARGO_HOME", "RUSTUP_HOME"] {
                    if let Ok(val) = std::env::var(var) { command.env(var, val); }
                }
            }
            command.output()
        }).await {
            Ok(Ok(out)) => out,
            _ => return None,
        };

        let stdout = String::from_utf8_lossy(&result.stdout).to_string();
        let stderr = String::from_utf8_lossy(&result.stderr).to_string();
        let combined = format!("{stdout}{stderr}").trim().to_string();

        if result.status.success() {
            let _ = ui_tx.send(UiUpdate::SystemMsg("[Marlin Verify] ✓ Tests passed.".into())).await;
            None
        } else {
            let snippet: String = combined.lines().rev().take(60)
                .collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            let msg = format!(
                "[Marlin Verify] Tests failed (exit {}). Fix the errors before continuing.\n\n{}",
                result.status.code().unwrap_or(-1),
                snippet
            );
            let _ = ui_tx.send(UiUpdate::SystemMsg(format!(
                "[Marlin Verify] ✗ Tests failed — injecting error into context."
            ))).await;
            Some(Message {
                role: "user".into(),
                content: msg,
                tool_calls: vec![],
                tool_use_id: String::new(),
                tool_call_id: String::new(),
                is_error: false,
            })
        }
    }

    /// LLM-based context compaction: summarize old turns when approaching token budget.
    async fn maybe_compact_history(&mut self, ui_tx: &mpsc::Sender<UiUpdate>) {
        const COMPACT_ABOVE: usize = 70_000;
        const KEEP_RECENT: usize = 8;

        let cur_tokens = estimate_tokens(&self.history, "");
        if cur_tokens < COMPACT_ABOVE { return; }
        if self.history.len() <= KEEP_RECENT { return; }
        // Don't re-compact immediately after a previous compaction; wait for 5k more tokens
        if self.compact_guard_tokens > 0 && cur_tokens < self.compact_guard_tokens + 5_000 {
            return;
        }

        let split = self.history.len() - KEEP_RECENT;
        let old: Vec<Message> = self.history[..split].to_vec();

        // Prefer the cheapest/fastest model so compaction doesn't waste quota
        let (compact_provider, compact_model) = self.cheapest_model();
        let provider = match self.registry.get(&compact_provider) {
            Ok(p) => p,
            Err(_) => return,
        };

        let ctx = compact_serialize(&old);

        let summary_req = StreamRequest {
            model: compact_model,
            messages: vec![Message {
                role: "user".into(),
                content: format!(
                    "Produce a dense technical summary of this coding session fragment for an AI \
                    coding assistant. Include: files created/modified (with key changes), commands \
                    run and their outcomes, errors encountered and how they were resolved, decisions \
                    made, and current task state. Be precise and comprehensive — this summary \
                    replaces the original turns in context.\n\n{ctx}"
                ),
                tool_calls: vec![],
                tool_use_id: String::new(),
                tool_call_id: String::new(),
                is_error: false,
            }],
            system_prompt: String::new(),
            max_tokens: 1500,
            tools: vec![],
        };

        let _ = ui_tx.send(UiUpdate::SystemMsg(
            format!("Compacting context (~{cur_tokens} tokens) — summarizing {split} older turns…")
        )).await;

        let mut stream = match provider.stream(summary_req).await {
            Ok(s) => s,
            Err(_) => return,
        };

        let mut summary = String::new();
        while let Some(chunk) = stream.recv().await {
            summary.push_str(&chunk.content);
            if chunk.done { break; }
        }

        if summary.trim().is_empty() { return; }

        let recent = self.history.split_off(split);
        self.history.clear();
        self.history.push(Message {
            role: "user".into(),
            content: format!("[Marlin Context Summary — {split} turns condensed]\n{}", summary.trim()),
            tool_calls: vec![],
            tool_use_id: String::new(),
            tool_call_id: String::new(),
            is_error: false,
        });
        self.history.extend(recent);

        let new_tokens = estimate_tokens(&self.history, "");
        self.compact_guard_tokens = new_tokens;

        let _ = ui_tx.send(UiUpdate::SystemMsg(
            format!("Context compacted: {split} turns → 1 summary (~{new_tokens} tokens now).")
        )).await;
    }

    /// Returns (provider, model) for cheap compaction calls, preferring haiku > sonnet > active.
    fn cheapest_model(&self) -> (String, String) {
        let p = &self.cfg.active_provider;
        if let Ok(prov) = self.registry.get(p) {
            let models = prov.models();
            if let Some(m) = models.iter().find(|m| m.contains("haiku")) {
                return (p.clone(), m.clone());
            }
            if let Some(m) = models.iter().find(|m| m.contains("sonnet")) {
                return (p.clone(), m.clone());
            }
        }
        (self.cfg.active_provider.clone(), self.cfg.active_model.clone())
    }

    // ── Model tier routing ────────────────────────────────────────────────────

    /// Select provider/model for this request based on difficulty score.
    async fn rate_and_route(&mut self, message: &str, ui_tx: &mpsc::Sender<UiUpdate>) {
        let Some(tiers) = self.cfg.model_tiers.clone() else {
            self.req_provider = self.cfg.active_provider.clone();
            self.req_model = self.cfg.active_model.clone();
            self.req_backup_provider.clear();
            self.req_backup_model.clear();
            return;
        };
        if !tiers.enabled {
            self.req_provider = self.cfg.active_provider.clone();
            self.req_model = self.cfg.active_model.clone();
            self.req_backup_provider.clear();
            self.req_backup_model.clear();
            return;
        }

        let score = self.rate_difficulty(message, &tiers).await;
        let tier_label = if score <= tiers.default_max_difficulty { "default" } else { "complex" };
        let _ = ui_tx.send(UiUpdate::TierSelected { score, tier: tier_label.into() }).await;

        let selected: &ModelTier = if score <= tiers.default_max_difficulty {
            &tiers.default
        } else {
            &tiers.complex
        };

        self.req_provider = selected.provider.clone();
        self.req_model = selected.model.clone();
        self.req_backup_provider = selected.backup_provider.clone();
        self.req_backup_model = selected.backup_model.clone();
    }

    /// Ask the rater model to score a task 1–100.
    async fn rate_difficulty(&self, message: &str, tiers: &crate::config::ModelTiers) -> u8 {
        let Ok(rater) = self.registry.get(&tiers.rater.provider) else {
            return 50;
        };
        let req = StreamRequest {
            model: tiers.rater.model.clone(),
            messages: vec![Message {
                role: "user".into(),
                content: format!(
                    "Rate the difficulty of this coding task from 1 to 100 where 1 is trivial \
                    and 100 is extremely complex architecture work. Reply with ONLY the number.\n\nTask: {message}"
                ),
                tool_calls: vec![],
                tool_use_id: String::new(),
                tool_call_id: String::new(),
                is_error: false,
            }],
            system_prompt: String::new(),
            max_tokens: 8,
            tools: vec![],
        };
        let mut text = String::new();
        if let Ok(mut stream) = rater.stream(req).await {
            while let Some(chunk) = stream.recv().await {
                text.push_str(&chunk.content);
                if chunk.done { break; }
            }
        }
        text.trim().parse::<u8>().unwrap_or(50).clamp(1, 100)
    }

    // ── Nightly daemon ────────────────────────────────────────────────────────

    fn maybe_spawn_daemon(&self, ui_tx: mpsc::Sender<UiUpdate>) {
        let Some(tiers) = &self.cfg.model_tiers else { return };
        let Ok(provider) = self.registry.get(&tiers.rater.provider) else { return };
        let model = tiers.rater.model.clone();
        skills::daemon::spawn(self.marlin_dir.clone(), provider, model, ui_tx);
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

        match &self.ast_mode {
            AstMode::Off => {}
            AstMode::SExpr => {
                s.push_str("\n## AST Context Mode: SEXPR\n");
                s.push_str("File reads are delivered as compact S-expression AST representations produced by `ast-compiler decompile --format sexpr`, not raw source text.\n");
                s.push_str("The root token is `(meta ...)` followed by the recursive node tree.\n");
                s.push_str("Parse the tree structurally when reasoning about code. When you need to write changes, use write_file or edit_file with reconstructed source text.\n");
            }
            AstMode::Harness => {
                s.push_str("\n## AST Context Mode: HARNESS\n");
                s.push_str("You have three specialized AST tools available. Prefer them over read_file/edit_file for all code understanding and mutation:\n");
                s.push_str("  ast_skeleton  <file>                  — API surface map (signatures, no bodies). Always start here.\n");
                s.push_str("  ast_get_node  <file> <node_id>        — Full JSON for one node. Use after skeleton to inspect a target.\n");
                s.push_str("  ast_mutate    <file> <node_id> <op>   — Structural edit + automatic recompile + optimize.\n\n");
                s.push_str("CRITICAL RULES:\n");
                s.push_str("  1. Do NOT use edit_file for code mutations — use ast_mutate instead.\n");
                s.push_str("  2. ast_mutate operations are: str-replace (old_json/new_json), append-stmt (statement_json), insert-before (index/statement_json).\n");
                s.push_str("  3. Always supply lang and source_file to ast_mutate so the source is regenerated deterministically.\n");
                s.push_str("  4. JSON values in node directives must be valid JSON, not source-code strings.\n");
            }
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
                if self.cfg.sandbox_mode == SandboxMode::Off && !self.cfg.skip_permissions && !self.is_allowed(rest) {
                    let first = rest.split_whitespace().next().unwrap_or(rest);
                    err!(format!("Command not allowed: {rest:?}\nUse /allow {first} or /sandbox [permissive|docker|gvisor]."));
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

            "/sandbox" => {
                match args.first().copied() {
                    Some("off") => {
                        self.cfg.sandbox_mode = SandboxMode::Off;
                        let _ = self.cfg.save();
                        sys!("Sandbox off — shell commands require /allow.");
                    }
                    Some("on") | Some("permissive") => {
                        self.cfg.sandbox_mode = SandboxMode::Permissive;
                        let _ = self.cfg.save();
                        sys!("Sandbox permissive — all commands allowed, running directly on host.");
                    }
                    Some("mxc") => {
                        if !executor::detect_mxc() {
                            err!(format!(
                                "MXC binary ({}) not found in PATH. \
                                Install from https://github.com/microsoft/mxc and retry.",
                                executor::mxc_binary_name()
                            ));
                        } else {
                            self.cfg.sandbox_mode = SandboxMode::Mxc;
                            let _ = self.cfg.save();
                            sys!(format!(
                                "Sandbox mxc — AI commands run via Microsoft eXecution Containers \
                                ({}, network blocked, only workdir mounted rw).",
                                executor::mxc_binary_name()
                            ));
                        }
                    }
                    _ => {
                        let mode = self.cfg.sandbox_mode.label();
                        let mxc = if executor::detect_mxc() {
                            format!("available ({})", executor::mxc_binary_name())
                        } else {
                            format!("not found ({})", executor::mxc_binary_name())
                        };
                        sys!(format!(
                            "Sandbox: {mode}  |  mxc: {mxc}\n\
                             /sandbox [off|permissive|mxc]"
                        ));
                    }
                }
            }

            "/permissions" => {
                match args.first().copied() {
                    Some("skip") => {
                        self.cfg.skip_permissions = true;
                        let _ = self.cfg.save();
                        sys!("Permissions skipped — all operations proceed without checks.");
                    }
                    Some("require") => {
                        self.cfg.skip_permissions = false;
                        let _ = self.cfg.save();
                        sys!("Permissions required — file and command checks enabled.");
                    }
                    _ => {
                        let state = if self.cfg.skip_permissions { "skip" } else { "require" };
                        sys!(format!("Permissions: {state}  (use /permissions skip|require)"));
                    }
                }
            }

            "/verify" => {
                if rest.is_empty() {
                    match &self.cfg.verify_command {
                        Some(cmd) => sys!(format!("Verify command: {cmd}  (use /verify off to clear)")),
                        None => sys!("No verify command set.  Usage: /verify <shell-command>"),
                    }
                } else if rest == "off" || rest == "none" {
                    self.cfg.verify_command = None;
                    let _ = self.cfg.save();
                    sys!("Verify command cleared.");
                } else {
                    self.cfg.verify_command = Some(rest.to_string());
                    let _ = self.cfg.save();
                    sys!(format!("Verify command set: {rest}"));
                }
            }

            "/ast" => {
                let new_mode = match args.first().copied() {
                    Some("off")     => Some(AstMode::Off),
                    Some("sexpr")   => Some(AstMode::SExpr),
                    Some("harness") => Some(AstMode::Harness),
                    Some(other) => {
                        err!(format!("Unknown AST mode {other:?} — use: off, sexpr, harness"));
                        return;
                    }
                    None => None,
                };
                if let Some(mode) = new_mode {
                    let label = mode.label();
                    self.ast_mode = mode.clone();
                    self.cfg.ast_mode = mode.clone();
                    let _ = self.cfg.save();
                    let _ = ui_tx.send(UiUpdate::AstMode(mode)).await;
                    match label {
                        "off"     => sys!("AST mode off — file reads use raw text."),
                        "sexpr"   => sys!("AST mode: SEXPR — file reads deliver compact S-expression ASTs via ast-compiler."),
                        "harness" => sys!("AST mode: HARNESS — ast_skeleton / ast_get_node / ast_mutate tools now active."),
                        _         => {}
                    }
                } else {
                    sys!(format!("AST mode: {}  (use /ast off|sexpr|harness)", self.ast_mode.label()));
                }
            }

            "/clean-env" => {
                match args.first().copied() {
                    Some("on") => {
                        self.cfg.clean_env = true;
                        let _ = self.cfg.save();
                        sys!("Clean-env sandboxing ON — subprocesses get a stripped environment.");
                    }
                    Some("off") => {
                        self.cfg.clean_env = false;
                        let _ = self.cfg.save();
                        sys!("Clean-env sandboxing OFF.");
                    }
                    _ => {
                        let state = if self.cfg.clean_env { "on" } else { "off" };
                        sys!(format!("Clean-env: {state}  (use /clean-env on|off)"));
                    }
                }
            }

            "/theme" => {
                match args.first().copied() {
                    Some("light") => {
                        self.cfg.theme = "light".into();
                        crate::tui::styles::set_light_theme(true);
                        let _ = self.cfg.save();
                        sys!("Theme set to light.");
                    }
                    Some("dark") => {
                        self.cfg.theme = "dark".into();
                        crate::tui::styles::set_light_theme(false);
                        let _ = self.cfg.save();
                        sys!("Theme set to dark.");
                    }
                    _ => {
                        sys!(format!("Theme: {}  (use /theme dark|light)", self.cfg.theme));
                    }
                }
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
                        let _ = ui_tx.send(UiUpdate::IndexBuilt).await;
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

            "/skill" | "/skills" => {
                let subcmd = args.first().copied().unwrap_or("list");
                let _subrest = args.get(1..).map(|a| a.join(" ")).unwrap_or_default();
                let subargs: Vec<&str> = args.get(1..).map(|a| a.to_vec()).unwrap_or_default();

                match subcmd {
                    "list" | "ls" => {
                        if self.skills.is_empty() {
                            sys!("No skills installed. Add TOML files to ~/.marlin/skills/");
                        } else {
                            let lines: Vec<String> = self.skills.iter().map(|s| {
                                format!("  {:20} — {}", s.name, s.description)
                            }).collect();
                            sys!(format!("Skills ({}):\n{}", self.skills.len(), lines.join("\n")));
                        }
                    }

                    "run" | "r" => {
                        if subargs.is_empty() {
                            sys!("Usage: /skill run <name> [query]");
                            return;
                        }
                        let skill_name = subargs[0];
                        let query = if subargs.len() > 1 {
                            subargs[1..].join(" ")
                        } else {
                            self.active_goal.clone()
                        };
                        if let Some(skill) = self.skills.iter().find(|s| s.name == skill_name).cloned() {
                            match skill.run.kind {
                                skills::SkillKind::Shell => {
                                    sys!(format!("Running skill '{}' with query: {query}", skill.name));
                                    match skills::executor::skill_command(&skill, &query) {
                                        Err(e) => err!(format!("Skill error: {e}")),
                                        Ok(cmd) => {
                                            let cmd_json = serde_json::json!({"command": cmd}).to_string();
                                            let wd = self.work_dir.clone();
                                            let clean_env = self.cfg.clean_env;
                                            let logs_dir = self.marlin_dir.join("logs");
                                            let result = tokio::task::spawn_blocking(move || {
                                                executor::execute(
                                                    "run_command",
                                                    &cmd_json,
                                                    &wd,
                                                    &|_| true,
                                                    None,
                                                    None,
                                                    Some(&logs_dir),
                                                    clean_env,
                                                    crate::config::AstMode::Off,
                                                    &crate::config::SandboxMode::Off,
                                                )
                                            }).await;
                                            match result {
                                                Ok(r) if r.is_error => err!(format!("[skill: {skill_name}]\n{}", r.output)),
                                                Ok(r)               => sys!(format!("[skill: {skill_name}]\n{}", r.output)),
                                                Err(e)              => err!(format!("Skill task panic: {e}")),
                                            }
                                        }
                                    }
                                }
                                skills::SkillKind::Prompt => {
                                    match skills::executor::expand_prompt(&skill, &query) {
                                        Ok(prompt) => {
                                            sys!(format!("[skill: {}] Expanded prompt — copy and send to run:\n\n{prompt}", skill.name));
                                        }
                                        Err(e) => err!(format!("Skill error: {e}")),
                                    }
                                }
                            }
                        } else {
                            err!(format!("Unknown skill '{skill_name}'.  Use /skill list."));
                        }
                    }

                    "new" | "create" => {
                        let name = if subargs.is_empty() { "my_skill" } else { subargs[0] };
                        let skill = skills::Skill {
                            name: name.to_string(),
                            description: "Describe what this skill does".into(),
                            triggers: vec!["keyword1".into(), "keyword2".into()],
                            run: skills::SkillRun {
                                kind: skills::SkillKind::Shell,
                                command: "echo {query}".into(),
                                template: String::new(),
                            },
                        };
                        match skills::save_skill(&self.marlin_dir, &skill) {
                            Ok(path) => {
                                sys!(format!("Skill template created:\n  {}\n\nEdit the file to customise it, then /skill reload.", path.display()));
                                self.skills = skills::load_all(&self.marlin_dir);
                            }
                            Err(e) => err!(format!("Failed to create skill: {e}")),
                        }
                    }

                    "suggest" => {
                        let suggestions_path = self.marlin_dir.join("skill_suggestions.md");
                        if suggestions_path.exists() {
                            match std::fs::read_to_string(&suggestions_path) {
                                Ok(content) => sys!(content),
                                Err(e) => err!(format!("Error reading suggestions: {e}")),
                            }
                        } else {
                            let context = self.history.iter().rev()
                                .find(|m| m.role == "user")
                                .map(|m| m.content.as_str())
                                .unwrap_or("");
                            let skill_defs: Vec<SkillDef> = self.skills.iter().map(SkillDef::from).collect();
                            let hits = skills::suggest::match_skills(context, &skill_defs);
                            if hits.is_empty() {
                                sys!("No skill suggestions yet. Nightly analysis runs after 20h of activity (requires model_tiers config).");
                            } else {
                                let lines: Vec<String> = hits.iter()
                                    .map(|m| format!("  {:20} — {}", m.name, m.description))
                                    .collect();
                                sys!(format!("Suggested skills:\n{}", lines.join("\n")));
                            }
                        }
                    }

                    "reload" => {
                        self.skills = skills::load_all(&self.marlin_dir);
                        let skill_defs: Vec<SkillDef> = self.skills.iter().map(SkillDef::from).collect();
                        let _ = ui_tx.send(UiUpdate::SkillsLoaded(skill_defs)).await;
                        sys!(format!("Reloaded {} skill(s).", self.skills.len()));
                    }

                    _ => {
                        sys!("Usage: /skill [list|run <name> [query]|new <name>|suggest|reload]");
                    }
                }
            }

            "/tiers" => {
                match args.first().copied() {
                    Some("on") => {
                        if self.cfg.model_tiers.is_none() {
                            self.cfg.model_tiers = Some(crate::config::ModelTiers::default());
                        }
                        self.cfg.model_tiers.as_mut().unwrap().enabled = true;
                        let _ = self.cfg.save();
                        sys!("Model tier routing enabled. Edit ~/.marlin/config.json (model_tiers) to configure.");
                    }
                    Some("off") => {
                        if let Some(t) = self.cfg.model_tiers.as_mut() { t.enabled = false; }
                        let _ = self.cfg.save();
                        sys!("Model tier routing disabled — using active_provider/active_model.");
                    }
                    _ => {
                        let state = self.cfg.model_tiers.as_ref()
                            .map(|t| if t.enabled {
                                format!(
                                    "enabled\n  default (≤{}): {} / {}\n  complex (>{}): {} / {}\n  rater: {} / {}",
                                    t.default_max_difficulty,
                                    t.default.provider, t.default.model,
                                    t.default_max_difficulty,
                                    t.complex.provider, t.complex.model,
                                    t.rater.provider, t.rater.model,
                                )
                            } else {
                                "disabled".into()
                            })
                            .unwrap_or_else(|| "not configured (use /tiers on to enable)".into());
                        sys!(format!("Model tiers: {state}\n\nUse /tiers on|off"));
                    }
                }
            }

            _ => {
                err!(format!("Unknown command: {cmd}  (type /help for list)"));
            }
        }
    }

    fn is_allowed(&self, cmd: &str) -> bool {
        policy::is_command_allowed(cmd, &self.allowed_commands)
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

fn extract_cmd_str(input_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input_json)
        .ok()
        .and_then(|v| v["command"].as_str().map(String::from))
        .unwrap_or_default()
}

fn is_destructive_cmd(cmd: &str) -> bool {
    const PATTERNS: &[&str] = &[
        "rm ", "rm -", "rmdir",
        "git push", "git reset --hard", "git clean", "git push -f", "git push --force",
        "kill ", "pkill", "killall",
        "shutdown", "reboot", "halt", "poweroff",
        "dd ", "mkfs", "fdisk",
        "DROP TABLE", "DROP DATABASE", "truncate",
        ":(){:|:&};:",  // fork bomb
    ];
    let lower = cmd.to_lowercase();
    PATTERNS.iter().any(|p| lower.contains(&p.to_lowercase()))
}

/// Serialize a slice of messages into a compact text block for the compaction LLM call.
/// Handles tool-call messages (content is often empty) by including the call list.
fn compact_serialize(messages: &[Message]) -> String {
    let mut out = String::new();
    for m in messages {
        match m.role.as_str() {
            "assistant" if !m.tool_calls.is_empty() => {
                if !m.content.is_empty() {
                    let snip: String = m.content.chars().take(300).collect();
                    out.push_str(&format!("[assistant]: {snip}\n"));
                }
                for tc in &m.tool_calls {
                    let input_snip: String = tc.input.chars().take(200).collect();
                    out.push_str(&format!("  [tool_call] {}({})\n", tc.name, input_snip));
                }
                out.push('\n');
            }
            "tool" => {
                let snip: String = m.content.chars().take(400).collect();
                out.push_str(&format!("  [tool_result]: {snip}\n\n"));
            }
            _ => {
                let snip: String = m.content.chars().take(600).collect();
                out.push_str(&format!("[{}]: {snip}\n\n", m.role));
            }
        }
    }
    out
}

fn tool_short_desc(name: &str, input_json: &str) -> String {
    let v = serde_json::from_str::<serde_json::Value>(input_json).unwrap_or_default();
    match name {
        "read_file" | "write_file" | "edit_file" => {
            let path = v["path"].as_str().unwrap_or("?");
            let basename = Path::new(path).file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string());
            format!("{name}: {basename}")
        }
        "run_command" => {
            let cmd = v["command"].as_str().unwrap_or("?");
            let short = cmd.split_whitespace().take(3).collect::<Vec<_>>().join(" ");
            format!("run: {short}")
        }
        "search_codebase" => {
            let q = v["query"].as_str().unwrap_or("?");
            format!("search: {q}")
        }
        "ast_skeleton" => {
            let f = v["file"].as_str().unwrap_or("?");
            format!("ast_skeleton: {}", Path::new(f).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| f.into()))
        }
        "ast_get_node" => {
            let id = v["node_id"].as_str().unwrap_or("?");
            format!("ast_get_node: {id}")
        }
        "ast_mutate" => {
            let op = v["operation"].as_str().unwrap_or("?");
            let id = v["node_id"].as_str().unwrap_or("?");
            format!("ast_mutate: {op} @ {id}")
        }
        _ => name.to_string(),
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
        ("/exec <cmd>", "run a shell command (must be /allow-ed first, or /sandbox on)"),
        ("/allow <prefix>", "allow a shell command prefix (e.g. /allow npm)"),
        ("/sandbox [off|permissive|mxc]", "command isolation: off=require /allow, permissive=allow all, mxc=MS eXecution Containers"),
        ("/permissions [skip|require]", "skip or require permission checks (persists)"),
        ("/verify [cmd|off]", "set shell command to run after every file edit (Write-Test-Fix)"),
        ("/ast [off|sexpr|harness]", "AST context mode: off=raw, sexpr=S-expr reads, harness=JSON surgery (persists)"),
        ("/clean-env [on|off]", "strip subprocess environment for isolation (persists)"),
        ("/theme [dark|light]", "switch UI theme (persists)"),
        ("/index [status]", "build (or check) the TF-IDF codebase search index"),
        ("/search <query>", "search the index and show ranked results with snippets"),
        ("/revert <file> [n]", "list file snapshots or restore one"),
        ("/resume", "resume the most recent saved session"),
        ("/history [n|clear]", "list saved sessions, load one by number, or clear all"),
        ("/cat <file>", "print file contents"),
        ("/ls [dir]", "list directory"),
        ("/cd <dir>", "change working directory"),
        ("/pwd", "show working directory"),
        ("/skill list", "list installed skills"),
        ("/skill run <name> [query]", "run a skill"),
        ("/skill new <name>", "create a new skill template"),
        ("/skill suggest", "show skill suggestions from nightly analysis"),
        ("/skill reload", "reload skills from disk"),
        ("/tiers [on|off]", "model tier routing (easy→default, hard→complex with backups)"),
    ];

    let mut s = "Commands:\n".to_string();
    for (cmd, desc) in &cmds {
        let pad = 32usize.saturating_sub(cmd.len());
        s.push_str(&format!("  {}{}{}\n", cmd, " ".repeat(pad.max(1)), desc));
    }
    s
}
