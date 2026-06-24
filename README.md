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

| Provider     | Command                              | Notes                     |
|--------------|--------------------------------------|---------------------------|
| Claude       | `/provider claude`                   | Anthropic, prompt caching |
| OpenRouter   | `/provider openrouter`               | 100+ models via one key   |
| Groq         | `/provider groq`                     | Very fast inference       |
| Ollama       | `/provider ollama`                   | Local, no key needed      |
| Fireworks    | `/provider fireworks`                |                           |
| Moonshot     | `/provider moonshot`                 |                           |
| Custom       | `/endpoint custom https://...`       | Any OpenAI-compat API     |

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

## Commands

```
/help                    show all commands
/clear                   clear chat history
/provider <name>         switch provider
/model <name>            switch model
/providers               list providers and their models
/models                  list models for current provider
/key <provider> <key>    set an API key
/endpoint <p> <url>      set a custom endpoint
/system <text>           add a system prompt
/tokens <n>              set max output tokens

/attach <file>           attach a file to your next message
/detach [file]           remove attachment(s)
/exec <cmd>              run a shell command (/allow-ed prefix required)
/allow <prefix>          whitelist a command prefix (e.g. /allow cargo)
/sandbox [on|off]        allow all commands autonomously (no allowlist needed)
/permissions [skip|req]  skip or require permission checks

/verify [cmd|off]        run a command after every file edit (Write-Test-Fix loop)
/clean-env [on|off]      strip subprocess environment for isolation

/index                   build the TF-IDF search index for this project
/index status            show index stats
/search <query>          search the index manually

/revert <file>           list snapshots for a file
/revert <file> <n>       restore snapshot n
/history                 list saved sessions
/history <n>             restore session n
/resume                  restore the most recent session

/theme [dark|light]      switch UI theme

/cat <file>              print file contents
/ls [dir]                list directory
/cd <dir>                change working directory
/pwd                     show working directory
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

On terminals ≥ 100 columns wide, a sidebar appears on the right with two panels:

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
Ratatui TUI    ←──  UiUpdate  (chunks, tool events, tasks, tokens)
               ──→  Action    (send, slash cmd, approve, deny)
                    Engine: agentic loop, provider SSE,
                            tool execution, context management
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

Stored at `~/.marlin/config.json`. All settings (keys, endpoints, provider, model, allowed commands, verify command, theme, sandbox flags) persist there automatically via slash commands — no manual editing needed.

---

## License

MIT
