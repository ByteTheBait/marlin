Output assets/demo.gif

Set Shell "zsh"
Set FontSize 14
Set Width 1400
Set Height 780
Set Padding 28
Set TypingSpeed 55ms
Set PlaybackSpeed 1.0
Set CursorBlink false

# ── cold open ────────────────────────────────────────────────────────────────
Sleep 400ms

# Launch marlin from the project root
Type "cd /Users/henry/Developer/marlin && ./target/release/marlin"
Enter
Sleep 300ms

# ── splash — tachyonfx sweep_in plays automatically ──────────────────────────
Sleep 2500ms

# press any key to skip into chat
Space
Sleep 600ms

# ── chat view — show slash-command autocomplete ───────────────────────────────
Type "/"
Sleep 600ms
Type "h"
Sleep 900ms
Tab
Sleep 250ms
Enter
Sleep 2200ms

# ── ask something real (Groq / qwen3-32b will respond) ───────────────────────
Type "list every .rs file in src/"
Enter
Sleep 9000ms

# ── graceful exit ─────────────────────────────────────────────────────────────
Ctrl+Q
