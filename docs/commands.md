# Commands, shortcuts & sidebar

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

See [Command permissions](security.md#command-permissions) for how `/allow` and `/sandbox` interact, and [Write-Test-Fix loop](security.md#write-test-fix-loop) for `/verify`.

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
| `→`        | Focus the sidebar           |
| `←` / `Esc`| Return to the text input    |

---

## Sidebar

On terminals 100+ columns wide, a sidebar appears on the right with three panels:

**Context Budget** — a live token-usage bar (exact counts when using Claude, heuristic otherwise). Turns yellow past 70%, red past 90%. At ~70k tokens Marlin automatically compacts old turns into an LLM summary; mechanical truncation kicks in at ~80k, and oldest turns are dropped at ~95k. See [Context management](architecture.md#context-management-token-based).

**Tasks** — a live task list showing every tool call made in the current goal, with status markers:

```
[x] read_file: main.rs        ← completed
[>] edit_file: auth.rs        ← in progress
[ ] run_command: cargo test   ← pending
[!] edit_file: lib.rs         ← failed
```

**Subagents** — every delegated skill run (see [Skills run as subagents](skills.md#skills-run-as-subagents)), with the same status markers plus the tool it's currently running:

```
[>] recent_commits: run_command   ← running, currently calling run_command
[x] make_skill                    ← finished successfully
[!] web_search                    ← finished with an error
```

Sidebar dimensions are configurable — see [Layout](extending.md#layout--marlinlayouttoml).
