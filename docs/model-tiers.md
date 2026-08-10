# Model tiers

Marlin can automatically route requests to different models based on task difficulty. Enable it:

```
/tiers on
```

Then edit `~/.marlin/config.json` to configure the tiers:

```json
"model_tiers": {
  "enabled": true,
  "default_max_difficulty": 40,
  "default": {
    "provider": "claude",
    "model": "claude-haiku-4-5",
    "backup_provider": "groq",
    "backup_model": "llama-3.3-70b-versatile"
  },
  "complex": {
    "provider": "claude",
    "model": "claude-sonnet-4-6",
    "backup_provider": "openrouter",
    "backup_model": "anthropic/claude-sonnet-4-6"
  },
  "rater": {
    "provider": "claude",
    "model": "claude-haiku-4-5"
  }
}
```

**How it works:**
1. Before each request, Marlin asks the rater model to score the task 1–100
2. Tasks scored ≤ `default_max_difficulty` go to the **default** tier (cheap, fast)
3. Tasks scored above the threshold go to the **complex** tier (powerful)
4. If the primary model is rate-limited and a backup is configured, Marlin switches immediately — no waiting

The current difficulty score and selected tier appear as a status message in chat.

When tiers are configured, the `default` tier also powers [skill subagents](skills.md#skills-run-as-subagents) and the [nightly skill-suggestion daemon](skills.md#nightly-skill-suggestions).
