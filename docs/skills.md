# Skills

Skills are reusable operations stored as `.qmd` files in `~/.marlin/skills/`. Five come built in.

Shell skills go through the same preflight funnel as every other shell-executing path — the real allow-list, sandbox mode, and destructive-command approval modal (see [Command permissions](security.md#command-permissions)). Only install skills you trust, but a locked-down config (`/sandbox off`, empty allow-list) applies to them too.

| Skill             | Triggers                                  | What it does                                        |
|-------------------|--------------------------------------------|-----------------------------------------------------|
| `explore`         | explore, structure, list files, tree      | Directory tree (excludes build/hidden)              |
| `web_search`      | search, look up, google, find online      | DuckDuckGo via curl                                 |
| `ripgrep`         | grep, rg, search code                     | `rg` across the working directory                   |
| `make_skill`      | create skill, new skill                   | Prompts the AI to write a new skill                 |
| `recent_commits`  | recent commits, git log, what changed     | `git log` + a summary — the chunk+prose combination |

## Using skills

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

## Skills run as subagents

By default, calling a skill doesn't run inline in the main conversation — it delegates to a **subagent**: a separate nested agent loop with its own message history and its own tools (the core file/shell/search tools, but not `run_skill` itself — no recursive delegation) that completes the task independently and reports back one final summary. The main model sees only that summary, not the subagent's intermediate tool calls or raw output, and its `run_skill` tool description says so explicitly, so it treats the summary as a trustworthy report rather than something to reflexively re-verify.

- Shell and combined (chunk+prose) skills instruct the subagent to run the exact resolved command(s) via `run_command` — deterministically, not paraphrased — then report on the output.
- Prompt-only skills hand the subagent the expanded template directly and let it act on it (e.g. `make_skill`'s subagent actually calls `write_file` itself, rather than just handing text back for the main model to act on).
- The subagent's tool calls go through the exact same preflight funnel as everything else (allow-list, sandbox mode, destructive-command approval modal) — delegating doesn't loosen anything.
- Subagent model: `model_tiers.default` when tiers are configured (a cheap/fast model doing the grunt work, independent of whether difficulty-based routing is enabled for the main conversation), otherwise whatever the main conversation is using.
- Running subagents (and their current tool call) appear in the sidebar under **Subagents**, below the task list — see [Sidebar](commands.md#sidebar).

This is the first step toward a longer-term direction where the main model acts purely as a manager — delegating all work to subagents rather than doing any of it directly, not just skills.

Toggle it off to go back to the old direct-execution behavior (skills run inline, raw output returned straight to the main model):

```
/subagents off        skills run inline again
/subagents on         back to subagent delegation (the default)
/subagents            show current state
```

Or set it permanently in `~/.marlin/config.json`:

```json
"skill_subagents": false
```

## Writing a skill

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

For the full built-in skills directory layout, see [Config](architecture.md#config).

## Nightly skill suggestions

When model tiers are configured, a background daemon runs once every 20 hours. It reads your recent sessions, asks the AI to suggest three new skills based on your workflow patterns, and saves them to `~/.marlin/skill_suggestions.md`. View them with `/skill suggest`. See [Model tiers](model-tiers.md) for configuring tiers.
