# Hacking Marlin

Marlin is built to be extended. Three layers, from least to most code. Every config-file category below (skills, commands, tools, providers, named themes) ships one real, working example on first run — write-once, so it never overwrites a file you've since edited or deleted — in addition to the extra examples shown here.

## Skills — qmd, no Rust required

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

Restart or run `/skill reload` and the skill appears in autocomplete and as a tool the LLM can call. See the [Skills](skills.md) doc for the full format (shell, prompt, and combined chunk+prose examples).

---

## Colors — `~/.marlin/theme.toml` and `~/.marlin/themes/`

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

## Custom slash commands — `~/.marlin/commands/`

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

## Custom LLM tools — `~/.marlin/tools/`

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

## Custom providers — `~/.marlin/providers/`

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

## Layout — `~/.marlin/layout.toml`

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

## Rust extension points

For tools, providers, and layout: prefer the TOML paths above — no recompile needed. Drop to Rust when you need behaviour that TOML can't express.

**Adding a new built-in LLM tool** (needs custom Rust logic):

1. `src/tools/mod.rs` — add a `ToolDef` entry to `all_tools()`.
2. `src/tools/executor.rs` — add a match arm to `execute()` before the `_ =>` fallback.
3. `src/tui/views/chat/helpers.rs` — optionally add a display name in `tool_display_name()` for the UI bubble.

**Adding a built-in slash command**:

1. `src/tui/widgets/suggestions.rs` — add a row to the `raw` array in `all_commands()`. Gives it tab-autocomplete.
2. `src/engine/mod.rs` — add a match arm in `handle_slash_command()` to parse and dispatch it.

**Adding a provider with a custom wire protocol**:

1. Add an impl in `src/providers/` (see `claude.rs` for the `Provider` trait).
2. Register it in `src/providers/registry.rs` → `Registry::new()`.
3. For OpenAI-compatible APIs, use the TOML path (`~/.marlin/providers/`) instead.

**Changing the layout**:

`src/tui/runner.rs` owns the frame split (chat area / sidebar). `src/tui/views/` holds the main panels; `src/tui/widgets/` holds sidebar, statusbar, and suggestion panel.
