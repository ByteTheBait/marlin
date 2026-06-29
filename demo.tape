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
Type "marlin""
Sleep 300ms
Enter

# ── splash — tachyonfx sweep_in plays automatically ──────────────────────────
Sleep 2500ms

# press any key to skip into chat
Space
Sleep 600ms

# ── chat view — show slash-command autocomplete ───────────────────────────────
Type "/"
Sleep 600ms
Type "help"
Sleep 900ms
Tab
Sleep 250ms
Enter
Sleep 2200ms

#  
Type "/sandb"
Sleep 500ms
Tab
Sleep 100ms
Type " "

Type "on"

Sleep 500ms

Enter
Type "/animate on"
Sleep 100ms
Enter
Sleep 750ms

# ── ask something real (Groq / qwen3-32b will respond) ───────────────────────
Type "explore this directory and list files"
Enter
Sleep 20s

# ── graceful exit ─────────────────────────────────────────────────────────────
Ctrl+Q
