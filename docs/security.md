# Command permissions & Write-Test-Fix

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

This applies uniformly across every shell-executing path — direct `run_command` calls, [skills](skills.md), external tools, and user-defined `/command`s all funnel through one preflight check. See [Safety](architecture.md#safety) for the implementation details.

---

## Write-Test-Fix loop

Set a verify command and Marlin will run it after every file edit, automatically feeding failures back to the model:

```
/verify cargo test
```

If tests fail, the last 60 lines of output are injected into the LLM's context and the agentic loop continues until they pass. Clear it with `/verify off`.
