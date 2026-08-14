//! A bounded, self-contained agentic loop for delegated work. Every skill
//! invocation (Phase: subagent delegation) spins one of these up instead of
//! running inline in the main conversation: its own message history, its own
//! provider call, its own tool execution — through the same preflight funnel
//! as everything else — and it reports back a single final summary rather
//! than raw tool output. This is the first step toward a "manager" main model
//! that delegates all grunt work instead of doing any of it directly.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::config::{AstMode, Config};
use crate::index::Index;
use crate::preflight;
use crate::providers::{Message, Provider, StreamRequest, ToolCall, ToolCallMsg};
use crate::tools::{all_tools, executor, policy};

use super::{Action, UiUpdate};

/// Default hard cap on tool-call round-trips within one subagent run —
/// independent of (and much tighter than) the main loop's cap, since a
/// subagent is meant to complete one narrow, delegated task. Overridable via
/// the config's `tool_call_limit` (see `run`).
const MAX_ITERATIONS: usize = 15;

pub struct SubagentResult {
    pub output: String,
    pub is_error: bool,
}

/// Tools available to a subagent: the core set only. No `run_skill` (never
/// let a subagent recursively spawn more subagents), no AST harness tools,
/// and no MCP tools (keep the nested loop's tool surface simple and
/// predictable, and its trust boundary equal to or narrower than the main
/// conversation's).
fn subagent_tools() -> Vec<crate::providers::ToolDef> {
    all_tools(&AstMode::Off, &[], &[], false, &[])
}

/// Build the task instructions handed to a skill's subagent, per skill shape:
/// shell/combined skills instruct the subagent to run the exact resolved
/// command(s) via `run_command` (deterministic — it doesn't get to paraphrase
/// them); prompt-only skills hand it the expanded template directly.
pub fn build_task(skill: &crate::skills::Skill, query: &str) -> Result<String, String> {
    if skill.is_shell() {
        let cmds = crate::skills::executor::resolve_chunks(skill, query).map_err(|e| e.to_string())?;
        let numbered = cmds.iter().enumerate()
            .map(|(i, c)| format!("{}. {c}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let mut task = format!(
            "Run the following shell command(s) via run_command, in order, EXACTLY as \
             given — do not paraphrase, combine, or otherwise modify them:\n\n{numbered}\n\n"
        );
        if skill.is_prompt() {
            let prose = crate::skills::executor::expand_prompt(skill, query).unwrap_or_default();
            task.push_str(&format!(
                "Then, using that output: {prose}\n\nReport a concise summary for the manager."
            ));
        } else {
            task.push_str("Report a concise summary of the output for the manager.");
        }
        Ok(task)
    } else if skill.is_prompt() {
        crate::skills::executor::expand_prompt(skill, query).map_err(|e| e.to_string())
    } else {
        Err(format!("skill '{}' has neither a shell chunk nor a prompt body", skill.name))
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    label: &str,
    instructions: &str,
    provider: Arc<dyn Provider>,
    model: &str,
    cfg: &Config,
    allowed: &[String],
    work_dir: &str,
    marlin_dir: &Path,
    code_index: Option<&Index>,
    ui_tx: &mpsc::Sender<UiUpdate>,
    action_rx: &mut mpsc::Receiver<Action>,
    cancel_flag: &Arc<AtomicBool>,
) -> SubagentResult {
    let id = uuid::Uuid::new_v4().to_string();
    let _ = ui_tx.send(UiUpdate::SubagentStarted { id: id.clone(), label: label.to_string() }).await;

    let system_prompt = format!(
        "You are a focused subagent completing one delegated task on behalf of a manager AI \
         and the user it serves. Use the available tools to accomplish the task directly — act, \
         don't ask for confirmation or explain what you could do. When finished (or if you \
         determine the task can't be completed), stop calling tools and reply with a concise \
         plain-text summary of what you did and any results the manager needs to know.\n\n\
         Working directory: {work_dir}"
    );

    let mut messages = vec![Message {
        role: "user".into(),
        content: instructions.to_string(),
        tool_calls: vec![],
        tool_use_id: String::new(),
        tool_call_id: String::new(),
        images: vec![],
        is_error: false,
    }];

    let tools = subagent_tools();
    let mut final_text = String::new();
    let mut ok = true;

    // Subagents are bounded by the same configurable tool-call limit as the
    // main loop (default 100), so a raised limit applies to delegated skill
    // runs too. MAX_ITERATIONS remains as the fallback default.
    let iterations = cfg.tool_call_limit.max(MAX_ITERATIONS);

    'iters: for _ in 0..iterations {
        if cancel_flag.load(Ordering::SeqCst) {
            final_text = "(cancelled)".into();
            ok = false;
            break;
        }

        let req = StreamRequest {
            model: model.to_string(),
            messages: messages.clone(),
            system_prompt: system_prompt.clone(),
            max_tokens: cfg.max_tokens,
            tools: tools.clone(),
            thinking: false,
        };

        let mut stream = match provider.stream(req).await {
            Ok(s) => s,
            Err(e) => {
                final_text = format!("subagent error: {e}");
                ok = false;
                break;
            }
        };

        let mut text_buf = String::new();
        let mut tool_calls: Vec<ToolCall> = vec![];

        loop {
            let Some(chunk) = stream.recv().await else { break };

            if let Some(e) = chunk.error {
                final_text = format!("subagent error: {e}");
                ok = false;
                break 'iters;
            }
            if chunk.retry_after > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(chunk.retry_after as u64)).await;
                continue;
            }
            if !chunk.content.is_empty() {
                text_buf.push_str(&chunk.content);
            }
            if chunk.done {
                tool_calls = chunk.tool_calls;
                break;
            }
        }

        if tool_calls.is_empty() {
            final_text = text_buf.trim().to_string();
            break;
        }

        messages.push(Message {
            role: "assistant".into(),
            content: text_buf.trim().to_string(),
            tool_calls: tool_calls.iter().map(|tc| ToolCallMsg {
                id: tc.id.clone(), name: tc.name.clone(), input: tc.input.clone(),
            }).collect(),
            tool_use_id: String::new(),
            tool_call_id: String::new(),
            images: vec![],
            is_error: false,
        });

        for tc in &tool_calls {
            let _ = ui_tx.send(UiUpdate::SubagentToolCall {
                id: id.clone(),
                name: tc.name.clone(),
            }).await;

            let result = run_one_tool(tc, cfg, allowed, work_dir, marlin_dir, code_index, ui_tx, action_rx).await;

            messages.push(Message {
                role: "tool".into(),
                content: result.output,
                tool_calls: vec![],
                tool_use_id: tc.id.clone(),
                tool_call_id: tc.id.clone(),
                images: vec![],
                is_error: result.is_error,
            });
        }
    }

    if final_text.is_empty() {
        final_text = if ok {
            "(subagent finished without a final summary)".into()
        } else {
            "(subagent stopped after repeated errors)".into()
        };
    }

    let _ = ui_tx.send(UiUpdate::SubagentFinished { id, ok }).await;

    SubagentResult { output: final_text, is_error: !ok }
}

/// Preflight-check and execute a single tool call from a subagent, exactly
/// like the main engine does for the top-level conversation — the same
/// allow-list, sandbox mode, and destructive-command approval modal apply.
#[allow(clippy::too_many_arguments)]
async fn run_one_tool(
    tc: &ToolCall,
    cfg: &Config,
    allowed: &[String],
    work_dir: &str,
    marlin_dir: &Path,
    code_index: Option<&Index>,
    ui_tx: &mpsc::Sender<UiUpdate>,
    action_rx: &mut mpsc::Receiver<Action>,
) -> executor::ToolResult {
    let verdict = match tc.name.as_str() {
        "run_command" => {
            let cmd = extract_cmd(&tc.input);
            preflight::check(&preflight::Invocation::shell("run_command", cmd), cfg, allowed)
        }
        "read_file" | "write_file" | "edit_file" | "notebook_edit" | "create_directory" => {
            match extract_path(&tc.input) {
                Some(path) => {
                    let resolved = executor::resolve_path(&path, work_dir);
                    preflight::check(&preflight::Invocation::paths(tc.name.clone(), vec![resolved]), cfg, allowed)
                }
                None => preflight::Verdict::Allow,
            }
        }
        _ => preflight::Verdict::Allow,
    };

    match verdict {
        preflight::Verdict::Deny(reason) => {
            return executor::ToolResult { output: reason, is_error: true };
        }
        preflight::Verdict::NeedApproval(reason) => {
            let _ = ui_tx.send(UiUpdate::AwaitingApproval { cmd: reason }).await;
            let approved = loop {
                match action_rx.recv().await {
                    Some(Action::Approve) => break true,
                    Some(Action::Deny) => break false,
                    Some(Action::CancelStream) => break false,
                    None => break false,
                    _ => {}
                }
            };
            if !approved {
                return executor::ToolResult { output: "Command denied by user.".into(), is_error: true };
            }
        }
        preflight::Verdict::Allow => {}
    }

    let name = tc.name.clone();
    let input = tc.input.clone();
    let work_dir = work_dir.to_string();
    let allowed = allowed.to_vec();
    let sandbox = cfg.sandbox_mode.allows_all() || cfg.skip_permissions;
    let sandbox_mode = cfg.sandbox_mode.clone();
    let clean_env = cfg.clean_env;
    let logs_dir = marlin_dir.join("logs");
    let idx_clone = code_index.cloned();

    tokio::task::spawn_blocking(move || {
        let search_fn: Option<Box<executor::SearchFn<'_>>> = idx_clone.clone().map(|idx| {
            let f: Box<executor::SearchFn<'_>> = Box::new(move |q: &str, lim: usize| {
                let results = crate::index::search(&idx, q, lim);
                crate::index::format_results(&results, q)
            });
            f
        });
        let symbol_search_fn: Option<Box<executor::SymbolSearchFn<'_>>> = idx_clone.map(|idx| {
            let f: Box<executor::SymbolSearchFn<'_>> = Box::new(move |sym: &str, lim: usize| {
                let results = crate::index::search_symbols(&idx, sym, lim);
                crate::index::format_symbol_results(&results, sym)
            });
            f
        });
        executor::execute(
            &name,
            &input,
            &work_dir,
            &|c: &str| sandbox || policy::is_command_allowed(c, &allowed),
            search_fn.as_deref(),
            symbol_search_fn.as_deref(),
            None,
            None, Some(&logs_dir),
            clean_env,
            AstMode::Off,
            &sandbox_mode,
            &[],
        )
    }).await.unwrap_or_else(|e| executor::ToolResult { output: e.to_string(), is_error: true })
}

fn extract_cmd(input_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input_json)
        .ok()
        .and_then(|v| v["command"].as_str().map(String::from))
        .unwrap_or_default()
}

fn extract_path(input_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(input_json)
        .ok()
        .and_then(|v| v["path"].as_str().map(String::from))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{Chunk, Skill, SkillFormat};

    fn shell_skill(cmd: &str) -> Skill {
        Skill {
            name: "test_skill".into(),
            description: String::new(),
            triggers: vec![],
            body: String::new(),
            chunks: vec![Chunk { lang: "sh".into(), source: cmd.into() }],
            format: SkillFormat::Qmd,
        }
    }

    fn prompt_skill(body: &str) -> Skill {
        Skill {
            name: "test_skill".into(),
            description: String::new(),
            triggers: vec![],
            body: body.into(),
            chunks: vec![],
            format: SkillFormat::Qmd,
        }
    }

    #[test]
    fn shell_only_task_instructs_exact_command() {
        let skill = shell_skill("rg {query} --heading");
        let task = build_task(&skill, "foo").unwrap();
        assert!(task.contains("rg 'foo' --heading"));
        assert!(task.contains("EXACTLY as"));
        assert!(task.contains("Report a concise summary"));
    }

    #[test]
    fn prompt_only_task_is_the_expanded_body() {
        let skill = prompt_skill("Do this: {input}");
        let task = build_task(&skill, "the thing").unwrap();
        assert_eq!(task, "Do this: the thing");
    }

    #[test]
    fn combined_task_runs_command_then_follows_prose() {
        let mut skill = shell_skill("git log --oneline -n 5");
        skill.body = "Summarize the themes above.".into();
        let task = build_task(&skill, "").unwrap();
        assert!(task.contains("git log --oneline -n 5"));
        assert!(task.contains("Then, using that output: Summarize the themes above."));
    }

    #[test]
    fn neither_shell_nor_prompt_is_an_error() {
        let skill = Skill {
            name: "empty".into(),
            description: String::new(),
            triggers: vec![],
            body: String::new(),
            chunks: vec![],
            format: SkillFormat::Qmd,
        };
        assert!(build_task(&skill, "x").is_err());
    }

    // ── end-to-end loop, against a fake provider (no network / no API cost) ──

    use std::sync::atomic::AtomicUsize;

    use async_trait::async_trait;

    use crate::config::Config;
    use crate::providers::StreamChunk;

    /// Returns one `run_command` tool call on its first invocation, then a
    /// plain final-text response with no tool calls on its second — enough to
    /// exercise the full loop: start event, one real tool execution through
    /// the preflight funnel, and a finish event.
    struct FakeProvider {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl Provider for FakeProvider {
        fn name(&self) -> &str { "fake" }
        fn models(&self) -> Vec<String> { vec!["fake-model".into()] }

        async fn stream(&self, _req: StreamRequest) -> anyhow::Result<mpsc::Receiver<StreamChunk>> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            let (tx, rx) = mpsc::channel(4);
            tokio::spawn(async move {
                let chunk = if n == 0 {
                    StreamChunk {
                        content: String::new(),
                        done: true,
                        error: None,
                        tool_calls: vec![ToolCall {
                            id: "call_1".into(),
                            name: "run_command".into(),
                            input: serde_json::json!({"command": "echo subagent_test_ok"}).to_string(),
                        }],
                        retry_after: 0,
                        rate_limit: None,
                    }
                } else {
                    StreamChunk {
                        content: "done: echoed the test string".into(),
                        done: true,
                        error: None,
                        tool_calls: vec![],
                        retry_after: 0,
                        rate_limit: None,
                    }
                };
                let _ = tx.send(chunk).await;
            });
            Ok(rx)
        }
    }

    #[tokio::test]
    async fn subagent_loop_runs_a_tool_call_and_returns_final_summary() {
        let mut cfg = Config::defaults();
        cfg.sandbox_mode = crate::config::SandboxMode::Permissive; // allow the echo without an approval prompt
        let allowed: Vec<String> = vec![];
        let work_dir = std::env::temp_dir().to_string_lossy().to_string();
        let marlin_dir = std::env::temp_dir().join("marlin_subagent_test");
        let _ = std::fs::create_dir_all(&marlin_dir);

        let (ui_tx, mut ui_rx) = mpsc::channel(32);
        let (_action_tx, mut action_rx) = mpsc::channel(4);
        let cancel_flag = Arc::new(AtomicBool::new(false));

        let provider: Arc<dyn Provider> = Arc::new(FakeProvider { calls: AtomicUsize::new(0) });

        let result = run(
            "test_skill",
            "Run echo subagent_test_ok via run_command and report the output.",
            provider,
            "fake-model",
            &cfg,
            &allowed,
            &work_dir,
            &marlin_dir,
            None,
            &ui_tx,
            &mut action_rx,
            &cancel_flag,
        ).await;

        assert!(!result.is_error, "subagent reported an error: {}", result.output);
        assert!(result.output.contains("echoed the test string"));

        drop(ui_tx);
        let mut events = vec![];
        while let Some(u) = ui_rx.recv().await {
            events.push(u);
        }
        assert!(matches!(events.first(), Some(UiUpdate::SubagentStarted { label, .. }) if label == "test_skill"));
        assert!(events.iter().any(|e| matches!(e, UiUpdate::SubagentToolCall { name, .. } if name == "run_command")));
        assert!(matches!(events.last(), Some(UiUpdate::SubagentFinished { ok: true, .. })));
    }
}
