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
Alternatively, run the install.sh script that bundles marlin, ast-compiler, and mxc:

```sh
# Cautious install

curl -fsSL https://raw.githubusercontent.com/pkill-mydaemons/marlin/main/install.sh -o install.sh # Pulls the file and stores it in a local one
vim install.sh # If you would like to see what content the file contains
./install.sh # Run it


# Or you can run it directly

curl -fsSL https://raw.githubusercontent.com/pkill-mydaemons/marlin/main/install.sh | bash

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

Skills are reusable operations stored as `.qmd` files in `~/.marlin/skills/`. Five come built in.

Shell skills go through the same preflight funnel as every other shell-executing path — the real allow-list, sandbox mode, and destructive-command approval modal (see [Command permissions](#command-permissions)). Only install skills you trust, but a locked-down config (`/sandbox off`, empty allow-list) applies to them too.

| Skill             | Triggers                                  | What it does                                        |
|-------------------|--------------------------------------------|-----------------------------------------------------|
| `explore`         | explore, structure, list files, tree      | Directory tree (excludes build/hidden)              |
| `web_search`      | search, look up, google, find online      | DuckDuckGo via curl                                 |
| `ripgrep`         | grep, rg, search code                     | `rg` across the working directory                   |
| `make_skill`      | create skill, new skill                   | Prompts the AI to write a new skill                 |
| `recent_commits`  | recent commits, git log, what changed     | `git log` + a summary — the chunk+prose combination |

### Using skills

```
/skill list                   list all installed skills
/skill run web_search rust async traits
/skill run ripgrep "fn main"
/skill new my_skill           create a template file to edit
/skill suggest                show AI-generated suggestions from nightly analysis
/skill reload                 reload skills from disk after editing
/skill migrate                rewrite deprecated .toml skills to .qmd
```

As you type, Marlin matches your message against skill trigger keywords and shows relevant skills in the suggestion panel — before you even send.

### Writing a skill

Drop a `.qmd` file in `~/.marlin/skills/` — YAML frontmatter for metadata, then optional prose, then an optional fenced shell chunk:

````qmd
---
name: gh_issues
description: List open GitHub issues for this repo
triggers: [issues, github, gh, open bugs]
---

List the open issues, then summarize them by priority.

```{sh}
gh issue list --limit 20
```
````

Or a prompt-only skill (no fenced chunk — the prose is expanded and fed back to the model instead of executed):

```qmd
---
name: explain_diff
description: Explain the current git diff
triggers: [explain diff, what changed, review changes]
---

Please explain this git diff clearly:

{input}
```

Or a skill with both — the chunk(s) run first, then the prose *and* their output are fed back to the model together:

````qmd
---
name: check_and_summarize
description: Run the test suite (optionally filtered) and summarize any failures
triggers: [check tests, run and summarize, test status]
---

The test output above is from the working directory. If anything failed,
explain the likely cause; if it all passed, just say so briefly.

```{sh}
cargo test {query} 2>&1 | tail -60
```
````

Use `{query}` (shell chunks) or `{input}` (prompt body) as the placeholder for user-supplied text — it's substituted as a single shell-quoted word in chunks, so leave it bare (`cmd {query}`, not `cmd '{query}'`). Chunks execute in order; nothing pipes from one into the next.

`.toml` skills (the pre-qmd format) still load but are deprecated — run `/skill migrate` to rewrite them to `.qmd`.

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
/allow <prefix>                    permit an executable or command prefix (e.g. /allow cargo test)
/sandbox [off|permissive|mxc]      command isolation mode (permissive allows all commands)
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

**Context Budget** — a live token-usage bar (exact counts when using Claude, heuristic otherwise). Turns yellow past 70%, red past 90%. At ~70k tokens Marlin automatically compacts old turns into an LLM summary; mechanical truncation kicks in at ~80k, and oldest turns are dropped at ~95k.

**Tasks** — a live task list showing every tool call made in the current goal, with status markers:

```
[x] read_file: main.rs        ← completed
[>] edit_file: auth.rs        ← in progress
[ ] run_command: cargo test   ← pending
[!] edit_file: lib.rs         ← failed
```

---

## Command permissions

Marlin enforces a two-layer permission model for shell commands:

**Allow list** — use `/allow <prefix>` to permit commands by executable name or command prefix:

```
/allow git          # permits: git status, git push, git log ...
/allow cargo test   # permits: cargo test, cargo test --release
                    # denies:  cargo build (different subcommand)
```

**Chain detection** — commands containing `&&`, `||`, `;`, backtick, or `$()` are always denied regardless of the allow list, because the second clause can be anything. Use `/sandbox permissive` or `"*"` in your allow list to lift this restriction.

```
git status          →  allowed (if git is allowed)
git log | head -20  →  allowed (pipes and redirects pass through)
git status && rm -rf .  →  denied (chain operator detected)
cargo test; curl evil.com  →  denied
```

**Sandbox modes** (set with `/sandbox`):
- `off` — default; commands require an explicit `/allow` entry
- `permissive` — all commands allowed, runs directly on the host
- `mxc` — runs commands inside an MXC isolation container

**Destructive command guard** — before running any shell command matching a destructive pattern (`rm`, `git push --force`, `kill`, `dd`, `DROP TABLE`, etc.), Marlin pauses regardless of allow status and shows a modal:

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
- At ~70k tokens: LLM compaction — old turns summarized into one block using the cheapest available model (haiku → sonnet → active); tool-call messages are serialized faithfully so the summary captures what was actually done
- At ~80k tokens: mechanical compression — tool results truncated first (highest token density, lowest value), then user/assistant messages trimmed to a tail snippet
- At ~95k tokens: oldest turns dropped, keeping the most recent 6 intact

Token counts use Claude's `POST /v1/messages/count_tokens` API for exact figures when the active provider is Claude; other providers fall back to a chars/4 heuristic.

**Loop guard:**
- Intercepts after 3 identical failing tool calls
- Tracks SHA-256 file hashes — warns the model if `edit_file` makes no actual change to a file

**Safety:**
- 100 tool-call cap per goal
- Every shell-executing path — the LLM's `run_command`, skills, external tools, and user-defined `/command`s — funnels through one preflight check (`src/preflight/mod.rs`): the real allow-list and sandbox mode, chain-operator detection (`&&`, `||`, `;`, backtick, `$()`), and a tokenized destructive-command classifier. None of them bypass `/allow` or `/sandbox`.
- Destructive command approval modal, shown for skills and user commands too, not just direct `run_command` calls
- File tool paths (`read_file`/`write_file`/`edit_file`/`create_directory`) must resolve within the working directory; an escape (e.g. `~/.ssh/authorized_keys`) requires the same approval modal
- Skill/tool/command placeholders (`{query}`, `{args}`, ...) are substituted as a single shell-quoted word, not raw string interpolation
- `/preflight [startup|skills|all]` reports missing binaries, unparsable config files, a stale index, and per-skill validation issues
- Optional `env_clear()` subprocess isolation (`/clean-env on`)
- Large command outputs (> 6k chars) spilled to `~/.marlin/logs/` with a pointer returned to the LLM

---

## Config

Stored at `~/.marlin/config.json`. All settings persist there automatically via slash commands.

Key directories:

```
~/.marlin/
  config.json          main config
  theme.toml           optional color overrides (see Hacking Marlin)
  layout.toml          sidebar dimensions (see Hacking Marlin)
  skills/              skill .qmd files (explore, web_search, ripgrep, make_skill built in); .toml also loads, deprecated
  commands/            user-defined slash commands (TOML files, /command reload)
  themes/              named theme files, selectable with /theme <name>
  tools/               user-defined LLM tools (TOML files, /tool reload)
  providers/           user-defined OpenAI-compatible providers (TOML files, restart to activate)
  skill_suggestions.md nightly AI skill recommendations
  sessions/            saved conversation history
  index/               TF-IDF search index
  logs/                large command output spill files
  snapshots/           per-file edit snapshots
```

---

## Hacking Marlin

Marlin is built to be extended. Three layers, from least to most code. Every config-file category below (skills, commands, tools, providers, named themes) ships one real, working example on first run — write-once, so it never overwrites a file you've since edited or deleted — in addition to the extra examples shown here.

### Skills — qmd, no Rust required

The fastest way to add new behavior. Drop a `.qmd` file in `~/.marlin/skills/`:

````qmd
---
name: gh_issues
description: List open GitHub issues for this repo
triggers: [issues, github, gh, open bugs]
---

```{sh}
gh issue list --limit 20
```
````

Restart or run `/skill reload` and the skill appears in autocomplete and as a tool the LLM can call. See the **Skills** section above for the full format (shell, prompt, and combined chunk+prose examples).

---

### Colors — `~/.marlin/theme.toml` and `~/.marlin/themes/`

**Quick override** — create `~/.marlin/theme.toml` to override any named color without recompiling. Each value is `[R, G, B]` (0–255). Omit a key to keep the built-in default. Applied at startup.

```toml
# ~/.marlin/theme.toml

[dark]
bg        = [8,   12,  24]   # app background
user      = [200, 215, 245]  # your messages
assistant = [0,   200, 200]  # marlin label / primary accent
system    = [100, 125, 150]  # timestamps and status text
success   = [70,  195, 110]  # ✓ indicators
error     = [215, 70,  70]   # ✗ errors
amber     = [215, 155, 45]   # tool call names
cobalt    = [40,  90,  210]  # command keys and badge backgrounds
steel     = [90,  120, 155]  # secondary text and arg labels

[light]
bg        = [252, 253, 255]
user      = [20,  45,  95]
assistant = [0,   110, 140]
system    = [95,  120, 150]
success   = [25,  125, 60]
error     = [175, 35,  35]
amber     = [150, 90,  15]
cobalt    = [25,  70,  165]
steel     = [75,  105, 145]
```

**Named themes** — drop `.toml` files into `~/.marlin/themes/` to create selectable themes. This exact `nord` theme ships as a working example on first run, so `~/.marlin/themes/nord.toml` already exists and `/theme nord` works out of the box:

```toml
# ~/.marlin/themes/nord.toml
name        = "nord"
description = "Nord color scheme"

[dark]
bg        = [46,  52,  64]
assistant = [136, 192, 208]
cobalt    = [94,  129, 172]
steel     = [76,  86,  106]
user      = [229, 233, 240]
```

**Minimal partial override** — every field is optional and falls back to the built-in default, so a single-color tweak is a one-liner. This only touches the error color in dark mode; everything else (light mode included) stays default:

```toml
# ~/.marlin/theme.toml
[dark]
error = [255, 90, 90]   # brighter red, easier to spot in a dim terminal
```

Switch to it at runtime — no restart needed:

```
/theme nord            apply a named theme
/theme dark            revert to built-in dark
/theme                 list available themes
```

Style functions live in `src/tui/styles.rs`. All colors route through named semantic functions (`style_user_text()`, `style_tool_badge()`, etc.) so a single palette change propagates everywhere.

---

### Custom slash commands — `~/.marlin/commands/`

Drop a `.toml` file into `~/.marlin/commands/` to register a new `/command`. A working `/status` example (`git status --short`) ships in this directory on first run.

```toml
# ~/.marlin/commands/deploy.toml
name        = "deploy"
description = "Deploy the current branch"
args        = "[--dry-run]"

[run]
type    = "shell"
command = "make deploy {args}"   # {args} = text typed after /deploy
```

Or a prompt-type command that injects text into the LLM conversation:

```toml
# ~/.marlin/commands/review.toml
name        = "review"
description = "Review the current git diff"

[run]
type     = "prompt"
template = "Review this diff carefully and explain every change:\n\n{input}"
```

A no-argument shell command works too — `{args}` is just an empty string if nothing follows the command name:

```toml
# ~/.marlin/commands/status.toml
name        = "status"
description = "Quick git + build status"

[run]
type    = "shell"
command = "git status --short && echo --- && cargo check --quiet"
```

After adding or editing a file:

```
/command reload        pick up changes without restarting
/command list          show all loaded commands
/command new deploy    create a template at ~/.marlin/commands/deploy.toml
```

User commands appear in tab-autocomplete alongside built-in ones. Shell commands run in the working directory. Prompt commands inject the expanded template into the LLM and run the agentic loop automatically.

---

### Custom LLM tools — `~/.marlin/tools/`

Drop a `.toml` file into `~/.marlin/tools/` to add a new function the model can call. A working `git_log` example (below) ships in this directory on first run.

```toml
# ~/.marlin/tools/run_tests.toml
name        = "run_tests"
description = "Run the test suite and return results"

[[properties]]
name        = "filter"
type        = "string"
description = "Optional test name filter"
required    = false

[run]
type    = "shell"
command = "cargo test {filter} 2>&1"
```

Property values from the LLM are substituted as `{name}` placeholders — one shell-quoted word each. An optional property the model doesn't supply resolves to an empty quoted string (`''`), not a stripped/missing argument, so leave a trailing `{name}` safe to include even when unused.

A tool with a required property and no optional ones — this is the one that ships by default:

```toml
# ~/.marlin/tools/git_log.toml
name        = "git_log"
description = "Show recent commits touching a given file"

[[properties]]
name        = "path"
type        = "string"
description = "File path relative to the repo root"
required    = true

[run]
type    = "shell"
command = "git log --oneline -n 10 -- {path}"
```

A tool with multiple properties, mixing required and optional:

```toml
# ~/.marlin/tools/http_get.toml
name        = "http_get"
description = "Fetch a URL and print the response body"

[[properties]]
name        = "url"
type        = "string"
description = "URL to fetch"
required    = true

[[properties]]
name        = "header"
type        = "string"
description = "Optional 'Key: Value' request header"
required    = false

[run]
type    = "shell"
command = "curl -sL -H {header} {url}"
```

```
/tool reload        pick up changes without restarting
/tool list          show all loaded tools
/tool new run_tests create a template at ~/.marlin/tools/run_tests.toml
```

---

### Custom providers — `~/.marlin/providers/`

Drop a `.toml` file into `~/.marlin/providers/` to add any OpenAI-compatible API:

```toml
# ~/.marlin/providers/mistral.toml
name     = "mistral"
endpoint = "https://api.mistral.ai/v1"
api_key  = "sk-..."           # or leave empty for local providers
model    = "mistral-large-latest"
models   = ["mistral-large-latest", "mistral-small-latest", "codestral-latest"]
```

A local server needs no key at all — this is how you'd point Marlin at LM Studio's OpenAI-compatible endpoint. This exact file ships by default (`~/.marlin/providers/lmstudio.toml`), so `/provider lmstudio` just works if you have LM Studio (or anything else serving that endpoint) running:

```toml
# ~/.marlin/providers/lmstudio.toml
name     = "lmstudio"
endpoint = "http://localhost:1234/v1"
model    = "local-model"
```

`models` is optional too — omit it and `/models` just falls back to the single `model` above:

```toml
# ~/.marlin/providers/deepseek.toml
name     = "deepseek"
endpoint = "https://api.deepseek.com/v1"
api_key  = "sk-..."
model    = "deepseek-chat"
```

Restart Marlin to activate. Once loaded, use `/provider mistral` to switch to it.

```
/provider list           show all active providers (* = current)
/provider new mistral    create a template at ~/.marlin/providers/mistral.toml
```

Built-in provider names (`claude`, `ollama`, `groq`, etc.) cannot be overridden by user files.

---

### Layout — `~/.marlin/layout.toml`

Control sidebar dimensions without recompiling:

```toml
# ~/.marlin/layout.toml
sidebar_width     = 34   # sidebar column width (default 34)
min_sidebar_width = 100  # minimum terminal width to show sidebar (default 100)
```

A wider sidebar for a big monitor, with more room for task lists and the token meter:

```toml
# ~/.marlin/layout.toml
sidebar_width     = 48
min_sidebar_width = 100
```

Effectively disable the sidebar on narrow terminals (or always, if you set it absurdly high) by raising the minimum width past any terminal you actually use:

```toml
# ~/.marlin/layout.toml
sidebar_width     = 34
min_sidebar_width = 999
```

Applied at startup. Delete the file to revert to defaults.

---

### Rust extension points

For tools, providers, and layout: prefer the TOML paths above — no recompile needed. Drop to Rust when you need behaviour that TOML can't express.

**Adding a new built-in LLM tool** (needs custom Rust logic):

1. `src/tools/mod.rs` — add a `ToolDef` entry to `all_tools()`.
2. `src/tools/executor.rs` — add a match arm to `execute()` before the `_ =>` fallback.
3. `src/tui/views/chat.rs` — optionally add a display name in `tool_display_name()` for the UI bubble.

**Adding a built-in slash command**:

1. `src/tui/widgets/suggestions.rs` — add a row to the `raw` array in `all_commands()`. Gives it tab-autocomplete.
2. `src/engine/mod.rs` — add a match arm in `handle_slash_command()` to parse and dispatch it.

**Adding a provider with a custom wire protocol**:

1. Add an impl in `src/providers/` (see `claude.rs` for the `Provider` trait).
2. Register it in `src/providers/registry.rs` → `Registry::new()`.
3. For OpenAI-compatible APIs, use the TOML path (`~/.marlin/providers/`) instead.

**Changing the layout**:

`src/tui/runner.rs` owns the frame split (chat area / sidebar). `src/tui/views/` holds the main panels; `src/tui/widgets/` holds sidebar, statusbar, and suggestion panel.

---

## License

MIT
