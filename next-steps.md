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

## Phase 5: Task Planning, Concurrency & Deferred TUI Views ✅ COMPLETE

### 5.1 Task Auto-Generation ✅
- `Engine::maybe_generate_plan()` asks the already-routed model for a short ordered
  plan before the tool loop starts
- Shown as a Pending checklist in the sidebar ("Plan" section, above the granular
  "Tasks" log); one step resolves to Completed/Failed per tool-call batch
- Best-effort — a failed/unparsable plan response just leaves it empty; `task_steps`
  tracking is unaffected either way

### 5.2 Parallel Task Groups ✅
- `execute_tools` spawns consecutive runs of parallel-safe calls (`read_file`,
  `list_directory`, `search_codebase`, `ast_skeleton`, `ast_get_node`) onto the
  blocking pool before awaiting any of them, instead of one at a time
- Writes, commands, skills, AST mutation, and external tools stay strictly sequential
- `TaskStep.parallel_group: Option<usize>` — steps that ran together render with a
  "∥" hint in the sidebar

### 5.3 Deferred TUI Views ✅
- `/view` and `/open` — read-only scrollable file pane (`ViewerPane`)
- `/diff-mode` — current file vs. its most recent snapshot, bounded LCS line diff
  (`DiffPane`, `snapshots::diff_lines`)
- `/edit` — editable pane (`EditorPane`, tui-textarea-driven), Ctrl+S routes through
  the same preflight funnel as the LLM's own `write_file` tool call; two-Esc guard
  before discarding unsaved changes
- All three render as overlays on top of chat, same pattern as the `/config` menu
- Note: tui-textarea 0.7.0 pins `ratatui 0.29`, incompatible with this project's
  `ratatui >=0.30` — `EditorPane` (and ChatView's own input box) hand-roll rendering
  rather than using tui-textarea's `Widget`/`set_block` API

### Already done before this phase (correcting earlier docs here)
- **Docker-equivalent sandboxing**: not bollard — `/sandbox mxc` already shells out to
  Microsoft eXecution Containers (`src/tools/executor.rs`), no outbound network, only
  the workdir mounted rw

## Remaining / Future Work

- Manual interactive verification of `/view`, `/edit`, `/diff-mode`, and the parallel-
  task sidebar grouping in a real terminal — this environment can't reliably drive
  TUI keystrokes, so Phase 5 was verified via unit tests + build/clippy/boot-smoke
  checks only, not by eyeballing the actual rendered panes
- `/edit` has no horizontal scroll for long lines (clipped, not wrapped)
- No file-browser behind `/open` — it's currently just an alias for `/view`
