# Next Steps — Implementation Status

## Phase 1: Token Thrift & Context Optimization ✅ COMPLETE

### 1.1 Token-Driven Context Management ✅
- `src/engine/context.rs` switched from character-based thresholds (320k–400k chars) to token-based limits (80k/100k tokens, ~3 chars/token estimate)
- Compress old messages to a tail snippet first, then drop oldest turns if still over budget
- Keep last 6 turns fully intact
- Token usage broadcast to sidebar via `UiUpdate::TokenUsage { used, budget }`

### 1.2 Output Truncation & Log Pointer ✅
- `src/tools/executor.rs`: `run_command` spills outputs > 6,000 chars to `~/.marlin/logs/cmd_<uuid>.log`
- Returns truncated message to LLM with last 40 lines as snippet and full log path

### 1.3 LLM-Based Context Compaction ✅
- `Engine::maybe_compact_history()` called at the top of every agentic loop iteration
- Triggers at 70k estimated tokens (before mechanical truncation at 80k)
- Summarizes old turns via the configured provider with a focused compaction prompt
- Replaces N old messages with a single `[Marlin Context Summary]` block
- Falls back to mechanical tail-truncation if LLM compaction fails

---

## Phase 2: Deterministic Loop Controls & Meta-Tracking ✅ COMPLETE

### 2.1 Task Tracking Matrix ✅
- `src/engine/tasks.rs`: `TaskStep` + `TaskStatus` (Pending/InProgress/Completed/Failed)
- Engine tracks every tool call as a task step, broadcasts `UiUpdate::TaskUpdate` after each batch
- Live task list displayed in sidebar panel with `[x]`/`[>]`/`[!]`/`[ ]` markers

### 2.2 Upgraded Loop Interceptor ✅
- `src/engine/loop_guard.rs`: SHA-256 file hashing via `sha2` crate
- `check_file_edit()`: when same file edited 2+ times with identical hash, injects targeted warning
- Warning: "You have edited X N times without changing its content. Do not retry the same edit…"

---

## Phase 3: Rigorous Closed-Loop Sandbox Verification ✅ COMPLETE

### 3.1 Write-Test-Fix State Machine ✅
- `/verify <cmd>` sets a shell command that runs after every successful `edit_file`/`write_file`
- Example: `/verify cargo test` or `/verify npm test`
- On failure: last 60 lines of output injected into history as a user message, forcing LLM to fix
- On success: `[Marlin Verify] ✓ Tests passed.` system message
- Persists to `~/.marlin/config.json` via `verify_command: Option<String>`
- `/verify off` clears it

### 3.2 Local Sandboxing ✅
- `/clean-env on` enables environment stripping for all subprocesses
- Uses `Command::env_clear()` then re-injects only: `PATH`, `HOME`, `USER`, `LANG`, `LC_ALL`, `CARGO_HOME`, `RUSTUP_HOME`, `GOPATH`, `NODE_PATH`, `npm_config_prefix`
- Persists via `clean_env: bool` in config
- Docker/bollard containerization remains deferred

---

## Phase 4: UI/UX Enhancements ✅ COMPLETE

### 4.1 Split Layout ✅
- Horizontal split — sidebar at 34 cols on terminals ≥ 100 wide

### 4.2 Context & Budget Monitor ✅
- Token budget bar in sidebar (color shifts yellow → red approaching limit)
- Shows `~Xk / 100k tokens` label

### 4.3 Task Tracking Matrix ✅
- Live tool call steps in sidebar with status markers

### 4.4 Human-in-the-Loop Modal ✅
- Destructive command patterns: `rm`, `git push --force`, `kill`, `dd`, `shutdown`, `DROP TABLE`, etc.
- Double red-border modal centered over chat; `y`/Enter approve, `n`/Esc deny
- Engine pauses agentic loop awaiting `Action::Approve` / `Action::Deny`

---

## Remaining / Future Work

- **Docker sandboxing**: Optional bollard crate integration to run commands in an isolated container
- **Task auto-generation**: Pre-plan generation via LLM before agentic loop (shows intended steps upfront rather than retrospectively)
- **Parallel task groups**: `parallel_group: Option<usize>` on TaskStep for concurrent tool visualization
