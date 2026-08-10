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
git clone https://github.com/ByteTheBait/marlin
cd marlin
cargo build --release
# optionally link to PATH
ln -sf $PWD/target/release/marlin /usr/local/bin/marlin
```
Alternatively, run the install.sh script that bundles marlin, ast-compiler, and mxc:

```sh
# Cautious install

curl -fsSL https://raw.githubusercontent.com/ByteTheBait/marlin/main/install.sh -o install.sh # Pulls the file and stores it in a local one
vim install.sh # If you would like to see what content the file contains
./install.sh # Run it


# Or you can run it directly

curl -fsSL https://raw.githubusercontent.com/ByteTheBait/marlin/main/install.sh | bash

```

**Requirements:** Rust 1.75+, a terminal that supports 24-bit color.

**Optional external tools** (required for [AST mode](docs/ast-mode.md)):
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
|--------------|--------------------------------------|---------------------------------------------|---------------------------|
| Claude       | `/provider claude`                   | claude-sonnet-4-5                          | Anthropic, prompt caching |
| OpenRouter   | `/provider openrouter`               | anthropic/claude-sonnet-4-5                | 100+ models via one key   |
| Groq         | `/provider groq`                     | llama-3.3-70b-versatile                    | Very fast inference       |
| Ollama       | `/provider ollama`                   | llama3                                     | Local, no key needed      |
| Fireworks    | `/provider fireworks`                | accounts/fireworks/models/llama-v3-70b-instruct |                      |
| Moonshot     | `/provider moonshot`                 | moonshot-v1-8k                             |                           |
| Custom       | `/endpoint custom https://...`       | default                                    | Any OpenAI-compat API     |

Switch model: `/model claude-opus-4-5` (or any model your provider supports). Bring your own OpenAI-compatible provider — see [Custom providers](docs/extending.md#custom-providers--marlinproviders).

---

## Documentation

- [Tools](docs/tools.md) — the built-in file/shell/search tools the LLM calls
- [Skills](docs/skills.md) — reusable `.qmd` operations, subagent delegation, nightly suggestions
- [Model tiers](docs/model-tiers.md) — difficulty-routed model selection
- [AST mode](docs/ast-mode.md) — structural file reads and edits
- [Commands, shortcuts & sidebar](docs/commands.md) — full slash-command reference
- [Command permissions & Write-Test-Fix](docs/security.md) — the allow-list/sandbox/destructive-command model
- [Codebase search & sessions](docs/search-and-sessions.md) — the TF-IDF index, session history, snapshots
- [Architecture & config](docs/architecture.md) — engine internals, context management, `~/.marlin/` layout
- [Hacking Marlin](docs/extending.md) — themes, custom commands/tools/providers, Rust extension points

---

## License

MIT
