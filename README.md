# Marlin

A terminal-native AI coding agent built in Go. Marlin runs entirely in your terminal, connects to any major AI provider, and can read, write, and edit your code autonomously — with built-in sandboxing, session history, file snapshots, and a full file browser.

---

## Features

- **Multi-provider** — Claude, Gemini, Groq, Ollama, Fireworks, Moonshot, or any OpenAI-compatible endpoint
- **Agentic tool loop** — AI reads files, writes files, edits files, runs commands, and browses directories until the goal is complete
- **File snapshots** — every file the AI touches is snapshotted before the edit; `/revert` any file to any prior state
- **Sandbox mode** — run the AI inside Docker or macOS `sandbox-exec`; it can execute commands freely without touching anything outside your project
- **Session history** — conversations auto-save and can be resumed with `/resume` or browsed with `/history`
- **Input history** — Up/Down arrow navigates previously sent messages (persisted across restarts)
- **File tree browser** — `/view` opens a full-screen file navigator; arrow keys browse, Enter opens files
- **Slash-command autocomplete** — type `/` to see all commands; Tab to complete
- **Built-in editor** — `/edit` opens a file in a full TUI editor with syntax-aware line numbers
- **Project context** — `info.json` keeps project name, description, run commands, and notes, injected into every AI prompt

---

## Install

**From source (requires Go 1.22+):**

```bash
git clone https://github.com/Pkill-MyDaemons/marlin
cd marlin
go build -o marlin .
./marlin
```

**Or install directly:**

```bash
go install github.com/Pkill-MyDaemons/marlin@latest
```

Config and data are stored in `~/.marlin/`.

---

## Provider Setup

Set an API key once; it's saved to `~/.marlin/config.json`.

| Provider | Command | Get a key |
|----------|---------|-----------|
| Claude (Anthropic) | `/key claude` | console.anthropic.com |
| Gemini (Google) | `/key gemini` | aistudio.google.com |
| Groq | `/key groq` | console.groq.com |
| Fireworks | `/key fireworks` | fireworks.ai |
| Moonshot | `/key moonshot` | platform.moonshot.cn |
| Ollama (local) | — no key needed — | ollama.ai |
| Custom OpenAI-compat | `/key custom` + `/endpoint custom <url>` | — |

Switch providers mid-session:

```
/provider groq
/model llama-3.3-70b-versatile
```

---

## Commands

### Chat & Navigation

| Command | Description |
|---------|-------------|
| `/help` | Show all commands |
| `/clear` | Clear chat history |
| `↑` / `↓` | Navigate input history (Up/Down arrow) |
| `Ctrl+C` | Cancel a running AI response |
| `Ctrl+Q` | Quit Marlin |

### Provider & Model

| Command | Description |
|---------|-------------|
| `/provider <name>` | Switch provider (`claude`, `gemini`, `groq`, `ollama`, `fireworks`, `moonshot`, `custom`) |
| `/p <name>` | Shorthand for `/provider` |
| `/model <name>` | Switch model |
| `/m <name>` | Shorthand for `/model` |
| `/providers` | List all providers and their models |
| `/models` | List models for the current provider |
| `/key <provider>` | Set API key (masked prompt, never hits shell history) |
| `/endpoint <provider> <url>` | Override API endpoint |
| `/tokens [n]` | Get or set max output tokens |

### Files & Editor

| Command | Description |
|---------|-------------|
| `/view [dir]` | Open the file tree browser |
| `/edit <file>` | Open a file in the built-in editor (read/write) |
| `/open <file>` | Open a file read-only |
| `/cat <file>` | Print file contents to chat |
| `/attach <file>` | Attach a file to your next message |
| `/a <file>` | Shorthand for `/attach` |
| `/detach [file]` | Remove attachment(s) |

### File Tree Browser (`/view`)

| Key | Action |
|-----|--------|
| `↑` / `↓` | Move cursor |
| `Enter` / `→` | Open file or enter directory |
| `←` / `Backspace` | Go up to parent (cursor returns to the folder you came from) |
| `PgUp` / `PgDn` | Jump a full page |
| `g` / `G` | Jump to top / bottom |
| `q` / `Esc` | Close and return to chat |

In file view mode, `←` / `Esc` / `q` go back to the tree. Markdown files are rendered; all other files show raw.

### File Snapshots & Revert

Marlin automatically snapshots every file **before** the AI modifies it. You can inspect and restore any prior version at any time. The working file is always the most recent version.

```
/revert src/main.go           # list all snapshots for that file
/revert src/main.go 1         # restore the most recent snapshot
/revert src/main.go 3         # restore a specific version
```

Restores are also snapshotted, so they're fully reversible too. Snapshots are stored in `~/.marlin/snapshots/` and never touch your project directory.

### Working Directory

| Command | Description |
|---------|-------------|
| `/pwd` | Show current working directory |
| `/cd <dir>` | Change working directory |
| `/ls [dir]` | List directory contents |

### Shell & Sandbox

| Command | Description |
|---------|-------------|
| `/exec <cmd>` | Run a shell command (must be `/allow`-ed first) |
| `/allow <prefix>` | Permit commands starting with prefix (e.g. `/allow npm`) |
| `/run [cmd]` | Run project command from `info.json` |
| `/sandbox [on\|off\|status]` | Toggle sandbox mode |

**Sandbox mode** starts an isolated Docker container (or macOS `sandbox-exec` if Docker isn't available) and routes all AI shell commands through it. The AI can run anything freely — installs, builds, test runners — without touching files outside your project. Enable it for unattended/autonomous runs:

```
/sandbox on
```

Docker must be running for the Docker backend. The `sandbox-exec` fallback works on macOS without any extra tools.

### Session History

| Command | Description |
|---------|-------------|
| `/resume` | Resume the most recent saved session |
| `/history` | List all saved sessions (newest first) |
| `/history <n>` | Load session number n (restores full AI context) |
| `/history clear` | Delete all saved sessions |

Sessions are auto-saved after every AI response to `~/.marlin/sessions/`. Loading a session restores the full conversation including the AI's tool-call history, so it can continue exactly where it left off.

### Project Info

| Command | Description |
|---------|-------------|
| `/init` | Create or re-create `info.json` via wizard |
| `/info` | View current `info.json` |
| `/info set {"key": "value"}` | Merge JSON into `info.json` |
| `/info notes <text>` | Update the project notes field |
| `/info edit` | Open `info.json` in the editor |
| `/system <prompt>` | Append extra instructions to the AI's system prompt |
| `/sys <prompt>` | Shorthand for `/system` |

`info.json` is injected into every AI prompt as project context. The AI can also update it automatically by embedding `<!--MARLIN:INFO {...} -->` blocks in its responses.

---

## How the AI Uses Tools

Marlin exposes six tools to the AI:

| Tool | What it does |
|------|-------------|
| `read_file` | Read any file |
| `write_file` | Create or overwrite a file |
| `edit_file` | Replace a specific string in a file (preferred for targeted edits) |
| `run_command` | Run a shell command (allow-listed or sandboxed) |
| `list_directory` | List directory contents |
| `create_directory` | Create a directory |

The AI runs in an **agentic loop**: it keeps calling tools until the goal is fully complete, then sends a plain-text response. There's no fixed step limit — it works until it's done (safety cap at 100 tool calls per goal). You can watch the progress in the status hint and cancel with `Ctrl+C` at any time.

---

## Rate Limiting

When a provider returns a `429 Too Many Requests` response, Marlin pauses and automatically resumes — no intervention needed.

- The status hint turns red and shows a live countdown with a progress bar:
  ```
  rate limited · resuming in 42s  [████████████░░░░░░░░]
  ```
- The wait duration is parsed from the provider's response headers. Marlin checks `Retry-After` first, then falls back to vendor-specific headers (`x-ratelimit-reset-requests`, `anthropic-ratelimit-requests-reset`, `x-ratelimit-reset`, etc.), taking the maximum so it waits until every limit has cleared. Headers can carry the wait as plain seconds, a Unix timestamp, an ISO 8601 datetime, an HTTP date, or a Go duration string — all are handled. If no header is present, Marlin defaults to 60 seconds.
- When the timer reaches zero, the request is automatically re-sent and the agentic loop continues exactly where it left off.
- Press `Ctrl+C` at any time to cancel the wait and abandon the current goal.

This means you can kick off a long autonomous task and walk away — if the provider rate-limits mid-way through, Marlin will sit patiently and then carry on without losing context.

---

## Copy & Paste

Marlin does not capture the mouse, so standard terminal text selection works normally — click and drag to select, then copy with your terminal's usual shortcut (`Cmd+C` on macOS, `Ctrl+Shift+C` on most Linux terminals).

---

## Data & Privacy

- Config: `~/.marlin/config.json` (API keys stored locally, mode `0600`)
- Sessions: `~/.marlin/sessions/`
- Snapshots: `~/.marlin/snapshots/`
- Input history: `~/.marlin/input_history.json`
- Nothing is sent anywhere except the active provider's API endpoint.

---

## License

MIT
