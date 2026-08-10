# Architecture & config

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

### Context management (token-based)

- At ~70k tokens: LLM compaction — old turns summarized into one block using the cheapest available model (haiku → sonnet → active); tool-call messages are serialized faithfully so the summary captures what was actually done
- At ~80k tokens: mechanical compression — tool results truncated first (highest token density, lowest value), then user/assistant messages trimmed to a tail snippet
- At ~95k tokens: oldest turns dropped, keeping the most recent 6 intact

Token counts use Claude's `POST /v1/messages/count_tokens` API for exact figures when the active provider is Claude; other providers fall back to a chars/4 heuristic.

### Loop guard

- Intercepts after 3 identical failing tool calls
- Tracks SHA-256 file hashes — warns the model if `edit_file` makes no actual change to a file

### Safety

- 100 tool-call cap per goal
- Every shell-executing path — the LLM's `run_command`, skills, external tools, and user-defined `/command`s — funnels through one preflight check (`src/preflight/mod.rs`): the real allow-list and sandbox mode, chain-operator detection (`&&`, `||`, `;`, backtick, `$()`), and a tokenized destructive-command classifier. None of them bypass `/allow` or `/sandbox`. See [Command permissions](security.md#command-permissions).
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

For how to populate `skills/`, `commands/`, `tools/`, `providers/`, `themes/`, and `layout.toml` yourself, see [Hacking Marlin](extending.md).
