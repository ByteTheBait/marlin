# Marlin: Safety Preflight, qmd Skills, and Prompt Budget

## Context

Marlin's extensibility story is already good — skills, slash commands, LLM tools, themes, layout, and providers all live as TOML under `~/.marlin/` with live `reload` commands. But the customization layer is exactly where the safety model breaks down, and an audit surfaced a real hole:

**User-defined skills and external tools bypass every guardrail.** `src/engine/mod.rs:620` passes `&|_| true` as the allow-list predicate and `:626` hardcodes `SandboxMode::Off` even when the user selected MXC. The same bypass is duplicated in the interactive `/skill run` path at `:1796` and `:1802`. `src/tools/external.rs:63` never consults the allow-list at all. Since `src/skills/executor.rs:12` pastes `{query}` verbatim into a shell string, the built-in `explore` skill (`D="{query}"`) executes arbitrary shell on a query of `.; rm -rf ~` — in the *default locked-down* config. This makes `/allow` and `/sandbox` cosmetic.

Three changes follow from that, plus the two the user asked for:

1. **Preflight** — a single mandatory funnel that every tool call passes through, plus startup environment checks and per-skill validation. Today the destructive-command check (`is_destructive_cmd`, `src/engine/mod.rs:1999`) is a substring match that never fires for skill-routed commands and misses `/bin/rm x` or `find . -delete`.
2. **qmd skills** — port `~/.marlin/skills/*.toml` to Quarto-style `.qmd`: YAML frontmatter for metadata, markdown prose as the prompt body, fenced `` ```{sh} `` chunks as the executable shell. This collapses today's awkward Shell-vs-Prompt `SkillKind` split into one file that reads like documentation.
3. **Prompt budget** — measured base injection is ~1,960 tokens of raw string literals (~1,328 in `src/tools/mod.rs` tool defs, ~629 in `effective_system_prompt`), and JSON schema overhead pushes the real figure to ~2.5–3k with AST harness on. Target is under 2k, enforced as a **warning, not a hard cap**.

Decisions already made with the user: qmd = Quarto/YAML-frontmatter markdown; token budget is a target that warns; preflight gates tool calls, startup environment, and skill validation (not pre-LLM-request).

---

## Phase A — Preflight core: close the bypass

New module `src/preflight/mod.rs`. One entry point every tool invocation funnels through:

```rust
pub enum Verdict { Allow, NeedApproval(String), Deny(String) }
pub fn check(inv: &Invocation, cfg: &Config, allowed: &[String]) -> Verdict
```

`Invocation` carries the *resolved* command (after placeholder substitution), the tool name, and any filesystem paths. Checks, in order:

- **Path containment** — every `read_file`/`write_file`/`edit_file`/`create_directory` path is resolved via the existing `resolve_path` (`src/tools/executor.rs:512`) and must stay within `work_dir`; escapes require approval. Today nothing stops `write_file` to `~/.ssh/authorized_keys`.
- **Chain operators** — reuse `policy::has_chain_operators` (`src/tools/policy.rs:14`), already correct and tested.
- **Allow-list** — reuse `policy::is_command_allowed` (`:43`).
- **Destructive classifier** — replace the substring `is_destructive_cmd` (`src/engine/mod.rs:1999`) with tokenized matching: split on whitespace/pipes, compare the *basename* of argv[0] against a deny set (`rm`, `dd`, `mkfs`, `shutdown`, …) so `/bin/rm` and `rm` both match, plus flag-aware rules (`find … -delete`, `git push --force`).
- **Sandbox routing** — honor the configured `SandboxMode` rather than hardcoding `Off`.

Then fix the call sites:

- `src/engine/mod.rs:620,626` and `:1796,1802` — pass the real `is_allowed` closure and real `sandbox_mode`.
- **Dedupe**: `execute_tools` (`:607-635`) and `/skill run` (`:1780-1822`) are near-identical copies. Collapse into one `run_shell_skill()` so a future fix can't miss a copy.
- `src/tools/external.rs:63` — route `ExternalTool::run` through `preflight::check` too.

**Substitution hardening** (this is the actual injection vector):

- `src/skills/executor.rs:12` and `src/tools/external.rs:65` do raw `String::replace`. Replace with shell-quoting: substitute each value as a single-quoted literal with embedded `'` escaped (the pattern already used correctly in `mxc_config_json`, `src/tools/executor.rs:404`).
- `src/tools/external.rs:70-79` walks the string deleting everything between any `{` and `}`. This silently guts `jq '{name: .x}'` and `awk '{print $1}'`. Replace with substitution of *declared property names only*; leave unknown braces untouched.
- Unify the two divergent `clean_env` variable allow-lists (`src/tools/executor.rs:159` keeps `NODE_PATH`/`npm_config_prefix`; `src/tools/external.rs:86` dropped them) into one shared `const CLEAN_ENV_VARS`.

Tests: `policy.rs` has 18 tests and they all pass, but they only cover the path that skills bypass. Add tests exercising `preflight::check` through the skill and external-tool entry points.

---

## Phase B — Preflight: startup + skill validation

**`preflight::startup()`** — runs in `Engine::new` (`src/engine/mod.rs:124-175`), renders a diagnostic panel:

- API key resolves for `active_provider`; provider endpoint reachable.
- Binaries present: `rg`, `gh`, `ast-compiler`, `ast-harness`, and MXC via the existing `executor::detect_mxc` (`src/tools/executor.rs:392`).
- Every `config.json`, `theme.toml`, `layout.toml`, and skill/tool/command/provider file parses — today parse failures are `eprintln!`'d into a TUI that has already taken over the terminal (`src/skills/mod.rs:79`), so they are invisible.
- Index freshness: `index/` mtime vs newest source file.

**`preflight::validate_skill()`** — on every load and reload:

- Frontmatter schema validation with the offending line surfaced.
- Placeholder consistency: every `{name}` in a chunk is a declared input, and every declared input is used.
- Shell lint on chunk bodies (shellcheck if present, else the chain-operator + destructive scan).
- Declared binary exists (first token of each chunk).
- Optional dry-run against a sentinel query.

Expose as `/preflight [startup|skills|all]`. Wire validation into `load_all` so a bad skill is reported and skipped, never silently dropped.

---

## Phase C — Port skills to qmd

New `src/skills/qmd.rs`. Target format:

```qmd
---
name: gh_issues
description: List open GitHub issues for this repo
triggers: [issues, github, gh, open bugs]
---

List the open issues, then summarize them by priority.

```{sh}
gh issue list --limit 20
```
```

Parser: split leading `---` frontmatter (YAML → serde), then walk the markdown body collecting fenced blocks whose info string matches `{sh}` / `{bash}`. Prose outside chunks is the prompt body.

**Model change** — retire the `SkillKind` Shell/Prompt enum (`src/skills/mod.rs:12-17`). A skill now has an optional `body: String` and `chunks: Vec<Chunk>`:

| chunks | body | behavior |
|---|---|---|
| yes | no | shell skill (today's `Shell`) |
| no | yes | prompt skill (today's `Prompt`) |
| yes | yes | run chunks, then feed prose + chunk output to the model |

That third row is new capability and the main reason the format is worth the churn.

**Blast radius is small** — the format is touched in exactly four places, all in `src/skills/mod.rs`: extension filter `:73`, parse `:77`, `save_skill` `:91`, `install_defaults` `:104`. The `/skill` dispatch arm (`src/engine/mod.rs:1752-1885`) is format-agnostic — it builds `Skill` structs and calls `save_skill`/`load_all`. Also update the `make_skill` built-in whose template string instructs the model to author TOML (`src/skills/mod.rs:176-190`).

**Migration** — keep parsing `.toml` for one release, marked deprecated; add `/skill migrate` to rewrite each into `.qmd`. `install_defaults` writes `.qmd` and skips a skill whose `.toml` twin exists.

**Dependency** — add `serde_yaml` (Cargo.toml has `toml` but no YAML parser).

**Non-goal**: chunk options (`echo`, `eval`, `label`) and output-chaining between chunks. The user chose the single-chunk-set interpretation, not the full literate-notebook one. Chunks execute in order; nothing pipes forward.

---

## Phase D — Prompt injection budget (target ~2k, warn)

**Measure exactly first.** `src/providers/claude.rs:288` already builds a token-count request body — reuse it to count the *assembled* system prompt + marshalled tool JSON, rather than the 4-chars/token estimate in `src/engine/context.rs:13`.

Add `/tokens` reporting a per-component breakdown: system prompt, each `ToolDef`, and the `run_skill` skill list.

**Reductions**, in descending value:

- **Drop the `## Tools` bullet list** from `effective_system_prompt` (`src/engine/mod.rs:966-980`). It restates every tool name and description that the API already receives as structured tool defs — pure duplication, ~150 tokens.
- **Stop enumerating all skills** in the `run_skill` description (`src/tools/mod.rs:88-98`). This grows O(n) with installed skills and is unbounded. Include only trigger-matched skills for the current turn via the existing `suggest::match_skills` (`src/skills/suggest.rs:12`), falling back to names-only (no descriptions) when nothing matches.
- **Tighten tool descriptions** in `src/tools/mod.rs` — `search_codebase` (`:75-77`) and `ast_mutate` (`:148-153`) are the two longest.
- AST harness tools (~380 tokens) are already conditionally injected (`:114`); verify SExpr mode doesn't pay for them.

**Enforcement** — a `budget` module computes the assembled size at request time. Over 2k: emit `UiUpdate` warning and a statusbar indicator. Never block. Add a test asserting the *default* configuration (no harness, no skills) stays under 2k so regressions are caught in CI.

---

## Phase E — Correctness fixes found in the audit

- **Panic**: `src/skills/daemon.rs:140` — `&content[..content.len().min(250)]` slices by byte offset and panics mid-UTF-8-char. Use `.chars().take(250)`. Audit the codebase for sibling `&s[..n]` slices (`src/tools/executor.rs:48` has the same shape).
- `src/engine/mod.rs:975` — `unwrap()` after `is_some()`; use `if let Some(idx)`.
- Stale default model IDs in `src/config/mod.rs` (`claude-sonnet-4-5`, `claude-sonnet-4-6`); current generation is Sonnet 5 / Haiku 4.5 / Opus 4.8.
- Clear the remaining ~25 clippy warnings, including three derivable `Default` impls (`src/config/mod.rs:82,109,295`).

---

## Verification

Each phase is independently checkable; run `cargo clippy` and `cargo test` throughout.

1. **Phase A is the one that must be proven end-to-end.** With a default config (`sandbox_mode = off`, empty `allowed_commands`), start Marlin and run `/skill run explore ".; touch /tmp/marlin_pwned"`. Before the fix this creates the file; after, preflight must deny it. Repeat via the LLM path (`run_skill` tool call) and via an external tool, since those are three distinct call sites.
2. Confirm `jq '{name: .x}'` survives external-tool substitution unmangled.
3. `/preflight all` on a machine missing `ast-compiler` reports it without crashing the TUI.
4. Write a skill with a deliberate frontmatter typo; confirm it's reported and skipped, not silently dropped.
5. Round-trip a `.toml` skill through `/skill migrate` and confirm behavior is identical.
6. `/tokens` on a default config reports under 2k; add ten skills and confirm the number stays bounded (proving the `run_skill` list no longer scales with skill count).
7. Feed the nightly daemon a session whose first user message begins with an emoji; confirm no panic.

Suggested order: **A → E → B → D → C**. A closes the security hole and E removes a live panic; B and D build on A's funnel; C is the largest diff and the easiest to defer.
