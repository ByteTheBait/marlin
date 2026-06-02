# Marlin

A terminal-native AI coding agent built in Go. Marlin runs entirely in your terminal, connects to any major AI provider, and can read, write, and edit your code autonomously — with built-in sandboxing, session history, file snapshots, a full file browser, and a suite of cost-control and safety features.

---

## Features

- **Multi-provider** — Claude, Gemini, Groq, Ollama, Fireworks, Moonshot, or any OpenAI-compatible endpoint
- **Agentic tool loop** — AI reads files, writes files, edits files, runs commands, and browses directories until the goal is complete
- **File snapshots** — every file the AI touches is snapshotted before the edit; `/revert` any file to any prior state
- **Interactive diff review** — `/diff-mode on` shows a coloured diff and asks for approval before any AI write lands on disk
- **Sandbox mode** — run the AI inside Docker or macOS `sandbox-exec`; it can execute commands freely without touching anything outside your project; sandbox state-diffs show newly generated files after each command
- **Session history** — conversations auto-save and can be resumed with `/resume` or browsed with `/history`
- **Input history** — Up/Down arrow navigates previously sent messages (persisted across restarts)
- **File tree browser** — `/view` opens a full-screen file navigator; arrow keys browse, Enter opens files
- **Slash-command autocomplete** — type `/` to see all commands; Tab to complete
- **Built-in editor** — `/edit` opens a file in a full TUI editor with syntax-aware line numbers
- **Project context** — `info.json` keeps project name, description, run commands, and notes, injected into every AI prompt
- **Smart context pruning** — compresses and drops old tool output automatically to stay within context limits
- **Proactive rate limiting** — pauses before a request that would exceed your token budget, preventing failed round-trips entirely

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
| `↑` / `↓` | Navigate input history (Up/Down arrow); falls through to viewport scroll when history is exhausted |
| `Ctrl+C` | Cancel a running AI response or rate-limit wait |
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

Marlin automatically snapshots every file **before** the AI modifies it. The working file is always the most recent version; you can restore any prior state at any time.

```
/revert src/main.go           # list all snapshots for that file
/revert src/main.go 1         # restore the most recent snapshot
/revert src/main.go 3         # restore a specific version
```

Restores are themselves snapshotted, so they're fully reversible. Snapshots live in `~/.marlin/snapshots/` and never touch your project directory.

### Interactive Diff Review

Enable diff mode to review every AI file write before it lands on disk:

```
/diff-mode on
```

When active, any `write_file` or `edit_file` call pauses the agentic loop and opens a full-screen coloured diff:

```
  src/auth.go  (change 1/2)   +12 -4

  @@ … 18 unchanged lines …
  - func validate(tok string) bool {
  + func validate(tok string) (Claims, error) {
  …

  a/y accept   r/n reject   q reject remaining   ↑↓ scroll
```

Rejected writes tell the AI the change was declined so it can revise. All writes still snapshot the original file regardless of diff mode.

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

**Sandbox mode** starts an isolated Docker container (or macOS `sandbox-exec` if Docker isn't available) and routes all AI shell commands through it. The AI can run anything freely — installs, builds, test runners — without touching files outside your project.

```
/sandbox on
```

After each sandboxed command, Marlin diffs the project directory (2 levels deep) and appends a notice if new files were generated:

```
[sandbox] new files: dist/bundle.js, swagger.json
```

The Docker backend mounts named volumes for common package-manager caches (`marlin-npm-cache`, `marlin-go-cache`, `marlin-pip-cache`, `marlin-cargo-cache`) so dependency installs are fast after the first run.

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
| `read_file` | Read any file. Pass `function` to extract just one named function or method instead of the whole file — dramatically cheaper on large codebases |
| `write_file` | Create or overwrite a file |
| `edit_file` | Replace a specific string in a file (preferred for targeted edits) |
| `run_command` | Run a shell command (allow-listed or sandboxed) |
| `list_directory` | List directory contents |
| `create_directory` | Create a directory |

The AI runs in an **agentic loop**: it keeps calling tools until the goal is fully complete, then sends a plain-text response. There's no fixed step limit — it works until it's done (safety cap at 100 tool calls per goal). The status hint shows the name of the currently-executing tool. Cancel with `Ctrl+C` at any time.

### Function Extraction

`read_file` accepts an optional `function` parameter to pull out just one symbol instead of the whole file:

```
read_file path="src/auth.go" function="validateToken"
→ // extracted: validateToken from auth.go (87 of 1 842 bytes)
  func validateToken(tok string) (Claims, error) { … }
```

| Language | Method |
|----------|--------|
| Go | `go/ast` — exact extraction including doc comments |
| Python | Indentation tracking (`def`, `async def`, `class`) |
| JS / TS / Rust / Java / C / C++ / Swift | Brace-counting line scanner |

Falls back to the full file with a warning if the symbol isn't found.

---

## Cost & Token Management

Marlin includes several layers of token-cost control.

### Prompt Caching (Claude)

When using Claude, the system prompt (including `info.json`) is sent as a cacheable block with `cache_control: ephemeral`. Anthropic serves it from cache on subsequent turns of the same tool loop, eliminating repeated billing for the same context.

### Proactive Rate-Limit Pause

After each successful response, Marlin records the remaining token/request budget from the provider's headers (`x-ratelimit-remaining-tokens`, `anthropic-ratelimit-tokens-remaining`, etc.). Before the next request, it estimates the outgoing payload size (~4 chars/token) and pauses if the estimate exceeds the remaining budget — preventing a wasted HTTP round-trip.

```
Proactive pause: ~12 400 tokens estimated, 5 000 remaining (window resets in 47s).
```

### Reactive Rate Limiting

If a `429` response arrives anyway, Marlin pauses and auto-resumes. The wait is read from `Retry-After` first, then vendor-specific reset headers, taking the maximum across all headers so it waits until every limit has cleared. Supported formats: plain seconds, Unix timestamp, ISO 8601 datetime, HTTP date, Go duration string. Defaults to 60 s if no header is present.

```
rate limited · resuming in 42s  [████████████░░░░░░░░]
```

`Ctrl+C` cancels either type of wait.

### Context Window Pruning

When the conversation history approaches the context ceiling (~320 000 chars), Marlin automatically compresses it in two phases:

1. **Compress** — any middle message over 2 000 chars is replaced with a summary line and its last 500 chars, preserving recent loop memory intact.
2. **Recursive drop** — if still over the 400 000-char hard limit after compression, the oldest middle messages are removed one at a time until the payload fits.

The last 6 messages are always kept verbatim. A notice appears in chat when pruning fires.

---

## Copy & Paste

Marlin does not capture the mouse, so standard terminal text selection works normally — click and drag to select, then copy with your terminal's usual shortcut (`Cmd+C` on macOS, `Ctrl+Shift+C` on most Linux terminals).

---

## Data & Privacy

| Path | Contents |
|------|----------|
| `~/.marlin/config.json` | Config and API keys (mode `0600`) |
| `~/.marlin/sessions/` | Auto-saved chat sessions |
| `~/.marlin/snapshots/` | Pre-edit file snapshots |
| `~/.marlin/input_history.json` | Sent-message history |

Nothing is sent anywhere except the active provider's API endpoint.

---

## License

MIT
