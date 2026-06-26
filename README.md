# marlin

An AI coding assistant that lives in your terminal.

![demo](assets/demo.gif)

---

## What it is

Marlin is a terminal chat interface for LLMs that can read, write, and execute code in your working directory. You describe what you want; it uses tools to get there — reading files, making edits, running commands, and searching the codebase — until the task is done.

Built in Rust on [Ratatui](https://ratatui.rs) with animated transitions via [tachyonfx](https://github.com/junkdog/tachyonfx).

---

## Install

```sh
git clone https://github.com/Pkill-MyDaemons/marlin
cd marlin
cargo build --release
# optionally link to PATH
ln -sf $PWD/target/release/marlin /usr/local/bin/marlin
```

**Requirements:** Rust 1.75+, a terminal that supports 24-bit color.

**Optional external tools** (required for AST mode):
- `ast-compiler` — for `/ast sexpr` (S-expression file reads)
- `ast-harness` — for `/ast harness` (structural JSON surgery)

---

## Quick start

```sh
cd your-project
marlin
```

Set your API key on first run:

```
/key claude sk-ant-...
/key openrouter sk-or-...
/provider claude
```

Then just talk to it:

```
> refactor the auth module to use JWT instead of sessions
> add tests for every exported function in src/utils.rs
> why is the build failing?
```

---

## Providers

| Provider     | Command                              | Default model                              | Notes                     |
|--------------|--------------------------------------|--------------------------------------------|---------------------------|
| Claude       | `/provider claude`                   | claude-sonnet-4-5                          | Anthropic, prompt caching |
| OpenRouter   | `/provider openrouter`               | anthropic/claude-sonnet-4-5                | 100+ models via one key   |
| Groq         | `/provider groq`                     | llama-3.3-70b-versatile                    | Very fast inference       |
| Ollama       | `/provider ollama`                   | llama3                                     | Local, no key needed      |
| Fireworks    | `/provider fireworks`                | accounts/fireworks/models/llama-v3-70b-instruct |                      |
| Moonshot     | `/provider moonshot`                 | moonshot-v1-8k                             |                           |
| Custom       | `/endpoint custom https://...`       | default                                    | Any OpenAI-compat API     |

Switch model: `/model claude-opus-4-5` (or any model your provider supports).

---

## Tools

Marlin calls these automatically — you don't invoke them manually:

| Tool               | What it does                                          |
|--------------------|-------------------------------------------------------|
| `read_file`        | Read a file (or just one function from a large file)  |
| `write_file`       | Create or overwrite a file                            |
| `edit_file`        | Targeted string replacement in a file                 |
| `run_command`      | Run a shell command                                   |
| `list_directory`   | List files and directories                            |
| `create_directory` | Create a directory                                    |
| `search_codebase`  | TF-IDF ranked search across the indexed project       |

Every file touched by an AI edit gets snapshotted first — use `/revert` to restore.

---

## Skills

Skills are reusable operations stored as TOML files in `~/.marlin/skills/`. Three come built in:

| Skill        | Triggers                                  | What it does                            |
|--------------|-------------------------------------------|-----------------------------------------|
| `explore`    | explore, structure, list files, tree      | Directory tree (excludes build/hidden)  |
| `web_search` | search, look up, google, find online      | DuckDuckGo via curl                     |
| `ripgrep`    | grep, rg, search code                     | `rg` across the working directory       |
| `make_skill` | create skill, new skill                   | Prompts the AI to write a new skill     |

### Using skills

```
/skill list                   list all installed skills
/skill run web_search rust async traits
/skill run ripgrep "fn main"
/skill new my_skill           create a template file to edit
/skill suggest                show AI-generated suggestions from nightly analysis
/skill reload                 reload skills from disk after editing
```

As you type, Marlin matches your message against skill trigger keywords and shows relevant skills in the suggestion panel — before you even send.

### Writing a skill

Drop a `.toml` file in `~/.marlin/skills/`:

```toml
name = "gh_issues"
description = "List open GitHub issues for this repo"
triggers = ["issues", "github", "gh", "open bugs"]

[run]
type = "shell"
command = "gh issue list --limit 20"
```

Or a prompt-type skill:

```toml
name = "explain_diff"
description = "Explain the current git diff"
triggers = ["explain diff", "what changed", "review changes"]

[run]
type = "prompt"
template = "Please explain this git diff clearly:\n\n{input}"
```

Use `{query}` (shell) or `{input}` (prompt) as the placeholder for user-supplied text.

### Nightly skill suggestions

When model tiers are configured, a background daemon runs once every 20 hours. It reads your recent sessions, asks the AI to suggest three new skills based on your workflow patterns, and saves them to `~/.marlin/skill_suggestions.md`. View them with `/skill suggest`.

---

## Model tiers

Marlin can automatically route requests to different models based on task difficulty. Enable it:

```
/tiers on
```

Then edit `~/.marlin/config.json` to configure the tiers:

```json
"model_tiers": {
  "enabled": true,
  "default_max_difficulty": 40,
  "default": {
    "provider": "claude",
    "model": "claude-haiku-4-5",
    "backup_provider": "groq",
    "backup_model": "llama-3.3-70b-versatile"
  },
  "complex": {
    "provider": "claude",
    "model": "claude-sonnet-4-6",
    "backup_provider": "openrouter",
    "backup_model": "anthropic/claude-sonnet-4-6"
  },
  "rater": {
    "provider": "claude",
    "model": "claude-haiku-4-5"
  }
}
```

**How it works:**
1. Before each request, Marlin asks the rater model to score the task 1–100
2. Tasks scored ≤ `default_max_difficulty` go to the **default** tier (cheap, fast)
3. Tasks scored above the threshold go to the **complex** tier (powerful)
4. If the primary model is rate-limited and a backup is configured, Marlin switches immediately — no waiting

The current difficulty score and selected tier appear as a status message in chat.

---

## AST Mode

AST mode changes how the LLM perceives and edits source files. Toggle it with `/ast`:

### `/ast off` (default)

Raw text mode. `read_file` returns the file's source as-is.

### `/ast sexpr`

Token-efficient exploration. `read_file` calls `ast-compiler decompile --format sexpr` instead of returning raw source, delivering a compact S-expression AST. Useful for navigating large files without burning through the context budget.

### `/ast harness`

Full structural surgery mode. Three additional LLM tools become active:

| Tool           | What it does                                                                 |
|----------------|------------------------------------------------------------------------------|
| `ast_skeleton` | Returns the API surface of a file (signatures only, no bodies) — start here  |
| `ast_get_node` | Returns full JSON for a single AST node by ID                                |
| `ast_mutate`   | Applies a structural edit to an AST node and auto-recompiles the source file |

`ast_mutate` supports three operations:

- **`str-replace`** — replace an AST node's JSON representation (`old_json` → `new_json`)
- **`append-stmt`** — append a statement JSON inside a node
- **`insert-before`** — insert a statement JSON before an index inside a node

After a mutation, Marlin automatically runs `ast-compiler compile` to regenerate the source file, then attempts an `optimize` pass. The LLM is instructed not to use `edit_file` while harness mode is active.

AST mode persists across sessions (stored in `~/.marlin/config.json`).

---

## Commands

```
/help                              show all commands
/clear                             clear chat history and attachments
/provider <name>                   switch provider
/model <name>                      switch model
/providers                         list all providers and their models
/models                            list models for current provider
/key <provider> <key>              set an API key
/endpoint <provider> <url>         set a custom API endpoint
/system <prompt>                   set additional system prompt
/tokens [n]                        get or set max output tokens

/attach <file>                     attach a file to your next message
/detach [file]                     remove attachment(s)
/exec <cmd>                        run a shell command (/allow-ed prefix required)
/allow <prefix>                    whitelist a command prefix (e.g. /allow cargo)
/sandbox [off|permissive|mxc]      command isolation mode
/permissions [skip|require]        skip or require permission checks (persists)

/verify [cmd|off]                  run a command after every file edit (Write-Test-Fix)
/ast [off|sexpr|harness]           AST context mode (persists)
/clean-env [on|off]                strip subprocess environment for isolation (persists)

/skill list                        list installed skills
/skill run <name> [query]          run a skill
/skill new <name>                  create a skill template
/skill suggest                     show nightly AI skill suggestions
/skill reload                      reload skills from disk

/tiers [on|off]                    model tier routing (easy→default, hard→complex)

/index                             build the TF-IDF search index for this project
/index status                      show index stats
/search <query>                    search the index manually

/revert <file>                     list snapshots for a file
/revert <file> <n>                 restore snapshot n
/history                           list saved sessions
/history <n>                       restore session n
/history clear                     delete all saved sessions
/resume                            restore the most recent session

/theme [dark|light]                switch UI theme (persists)
/cat <file>                        print file contents
/ls [dir]                          list directory
/cd <dir>                          change working directory
/pwd                               show working directory
```

---

## Keyboard shortcuts

| Key        | Action                      |
|------------|-----------------------------|
| `Enter`    | Send message                |
| `Ctrl+J`   | Insert newline              |
| `Ctrl+C`   | Cancel streaming response   |
| `Ctrl+Q`   | Quit                        |
| `↑` / `↓`  | Scroll history / chat       |
| `PgUp/Dn`  | Scroll chat by page         |
| `Tab`      | Autocomplete slash command  |

---

## Sidebar

On terminals 100+ columns wide, a sidebar appears on the right with two panels:

**Context Budget** — a live token-usage bar. Turns yellow past 70%, red past 90%. When the bar fills, Marlin automatically compacts old turns into a summary via the LLM before falling back to mechanical truncation.

**Tasks** — a live task list showing every tool call made in the current goal, with status markers:

```
[x] read_file: main.rs        ← completed
[>] edit_file: auth.rs        ← in progress
[ ] run_command: cargo test   ← pending
[!] edit_file: lib.rs         ← failed
```

---

## Destructive command guard

Before running any shell command matching a destructive pattern (`rm`, `git push --force`, `kill`, `dd`, `DROP TABLE`, etc.), Marlin pauses and shows a modal:

```
╔══ ⚠  Destructive Command ══╗
║  rm -rf ./dist              ║
║                             ║
║  Allow this command to run? ║
║  [y] Yes    [n] No          ║
╚═════════════════════════════╝
```

Press `y` to approve or `n` to deny. The engine resumes immediately either way.

---

## Write-Test-Fix loop

Set a verify command and Marlin will run it after every file edit, automatically feeding failures back to the model:

```
/verify cargo test
```

If tests fail, the last 60 lines of output are injected into the LLM's context and the agentic loop continues until they pass. Clear it with `/verify off`.

---

## Codebase search

Run `/index` once to build a TF-IDF index of your project. After that, Marlin automatically searches it before reading files, so it can navigate large codebases without reading every file.

```
/index            # build
/index status     # check stats
/search auth jwt  # manual search
```

The index is saved to `~/.marlin/index/` and updated automatically when Marlin writes or edits a file.

---

## Sessions & snapshots

Conversations are saved automatically at the end of each goal. Restore the last one with `/resume` or browse with `/history`.

File snapshots are taken before every AI edit. If something goes wrong:

```
/revert src/main.rs        # list snapshots
/revert src/main.rs 1      # restore the most recent one
```

---

## Architecture

Two threads, clean separation:

```
main thread          Tokio thread
──────────           ──────────────────────────────────────────
Ratatui TUI    ←──  UiUpdate  (chunks, tool events, tasks, tokens, skills)
               ──→  Action    (send, slash cmd, approve, deny)
                    Engine: agentic loop, provider SSE,
                            tool execution, context management,
                            skill runner, tier routing, nightly daemon
```

**Context management (token-based):**
- At ~70k tokens: LLM compaction — old turns summarized into one block via the configured provider
- At ~80k tokens: mechanical compression — long messages trimmed to a tail snippet
- At ~100k tokens: oldest turns dropped, keeping the last 8 intact

**Loop guard:**
- Intercepts after 3 identical failing tool calls
- Tracks SHA-256 file hashes — warns the model if `edit_file` makes no actual change to a file

**Safety:**
- 100 tool-call cap per goal
- Destructive command approval modal
- Optional `env_clear()` subprocess isolation (`/clean-env on`)
- Large command outputs (> 6k chars) spilled to `~/.marlin/logs/` with a pointer returned to the LLM

---

## Config

Stored at `~/.marlin/config.json`. All settings persist there automatically via slash commands.

Key directories:

```
~/.marlin/
  config.json          main config
  skills/              skill TOML files (explore, web_search, ripgrep, make_skill built in)
  skill_suggestions.md nightly AI skill recommendations
  sessions/            saved conversation history
  index/               TF-IDF search index
  logs/                large command output spill files
  snapshots/           per-file edit snapshots
```

---

## License

MIT
