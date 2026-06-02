package ui

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/bubbles/textinput"
	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/glamour"
	"github.com/charmbracelet/lipgloss"
	"marlin/internal/config"
	"marlin/internal/container"
	"marlin/internal/history"
	infomod "marlin/internal/info"
	"marlin/internal/providers"
	"marlin/internal/snapshots"
	"marlin/internal/tools"
)

// reInfoBlock matches <!--MARLIN:INFO ... --> blocks the AI may emit to update info.json.
var reInfoBlock = regexp.MustCompile(`(?s)<!--MARLIN:INFO\s*(\{.*?\})\s*-->`)

// Messages exchanged with the parent app.
type OpenEditorMsg struct {
	Filename string
	Readonly bool
}

type SetStatusMsg struct {
	Provider       string
	Model          string
	Info           string
	SandboxActive  bool
	SandboxBackend string
}

// Internal messages.
type streamChunkMsg struct {
	content    string
	done       bool
	err        error
	toolCalls  []providers.ToolCall
	retryAfter int // >0 means rate-limited
	rateLimit  *providers.RateLimitState
}

type toolExecResult struct {
	call    providers.ToolCall
	output  string
	isError bool
}

type toolsExecutedMsg struct {
	results []toolExecResult
}

type rateLimitTickMsg struct{}

type cmdOutputMsg struct {
	label  string
	output string
	err    error
}

type attachFileMsg struct {
	filename string
	content  string
	lines    int
}

type sandboxReadyMsg struct {
	mgr *container.Manager
	err error
}

type chatEntry struct {
	role      string // "user"|"assistant"|"system"|"error"|"output"|"tool_call"|"tool_result"
	content   string
	time      time.Time
	toolName  string // for tool_call entries
	isError   bool   // for tool_result entries
}

type chatInputMode int

const (
	modeNormal   chatInputMode = iota
	modeKeyInput               // masked API key entry
)

type setupStep int

const (
	setupDone    setupStep = iota
	setupAskName            // first launch: ask project name
	setupAskDesc            // ask description
	setupAskRun             // ask run command (skippable)
)

type attachment struct {
	filename string
	content  string
	lines    int
}

// cmdDef is a single slash-command entry shown in the autocomplete panel.
type cmdDef struct {
	cmd  string // e.g. "/help"
	args string // e.g. "<file>" — shown dimmed after command
	desc string
}

// cmdSuggestion is a cmdDef with an exact-match flag for bold rendering.
type cmdSuggestion struct {
	cmdDef
	exact bool
}

var allCommandDefs = []cmdDef{
	{"/help", "", "show all commands"},
	{"/clear", "", "clear chat history"},
	{"/provider", "<name>", "switch provider"},
	{"/p", "<name>", "switch provider (short)"},
	{"/model", "<name>", "switch model"},
	{"/m", "<name>", "switch model (short)"},
	{"/providers", "", "list all providers"},
	{"/models", "", "list models for current provider"},
	{"/key", "<provider>", "set API key (masked)"},
	{"/endpoint", "<provider> <url>", "set API endpoint"},
	{"/system", "<prompt>", "set additional system prompt"},
	{"/sys", "<prompt>", "set additional system prompt (short)"},
	{"/tokens", "[n]", "get/set max output tokens"},
	{"/info", "", "view info.json"},
	{"/info set", "{json}", "merge JSON into info.json"},
	{"/info notes", "<text>", "update notes in info.json"},
	{"/info edit", "", "open info.json in editor"},
	{"/init", "", "create/re-create info.json via wizard"},
	{"/attach", "<file>", "attach file to next message"},
	{"/a", "<file>", "attach file (short)"},
	{"/detach", "[file]", "remove attachment(s)"},
	{"/run", "[cmd]", "run project command from info.json"},
	{"/exec", "<cmd>", "run shell command (must be /allow-ed)"},
	{"/allow", "<prefix>", "allow a shell command prefix"},
	{"/sandbox", "[on|off|status]", "toggle isolated sandbox mode"},
	{"/revert", "<file> [n]", "show or restore file snapshots"},
	{"/resume", "", "resume the most recent session"},
	{"/history", "[n|clear]", "list or load saved sessions"},
	{"/edit", "<file>", "open file in editor"},
	{"/view", "[dir]", "browse file tree (arrow keys, Enter to open)"},
	{"/open", "<file>", "open file read-only in editor"},
	{"/cat", "<file>", "print file contents"},
	{"/ls", "[dir]", "list directory"},
	{"/cd", "<dir>", "change working directory"},
	{"/pwd", "", "show working directory"},
}

type ChatModel struct {
	cfg      *config.Config
	registry *providers.Registry
	workDir  string

	// project info
	projectInfo *infomod.Info
	setupStep   setupStep
	setupName   string
	setupDesc   string

	entries  []chatEntry
	history  []providers.Message

	viewport viewport.Model
	textarea textarea.Model
	renderer *glamour.TermRenderer

	// masked key input
	inputMode   chatInputMode
	keyProvider string
	keyInput    textinput.Model

	// pending file attachments
	attachments []attachment

	streaming      bool
	streamBuf      string
	streamCh       <-chan providers.StreamChunk
	cancelStream   context.CancelFunc
	toolIterations int // total tool calls fired for the current goal

	// goal tracking
	activeGoal string // the original user message driving the current agentic run

	// rate-limit auto-resume
	rateLimited    bool
	rateLimitSecs  int // countdown seconds remaining
	rateLimitTotal int // total wait (for display)

	// viewport auto-scroll: true = follow new content; false = user has scrolled up
	atBottom bool

	// proactive rate-limit state from the most recent successful response
	rlState *providers.RateLimitState

	// sandbox / container mode
	sandboxMode    bool
	sandboxManager *container.Manager

	// slash-command autocomplete
	suggestions []cmdSuggestion

	// input + session history
	inputHistory *history.InputHistory
	historyIdx   int    // -1 = not browsing; ≥0 = index into inputHistory.Entries
	historyDraft string // textarea draft saved when entering history mode
	session      *history.Session
	marlinDir    string

	width  int
	height int
}

func newKeyInput() textinput.Model {
	ti := textinput.New()
	ti.EchoMode = textinput.EchoPassword
	ti.EchoCharacter = '•'
	ti.Placeholder = "paste key here, Enter to confirm, Esc to cancel"
	ti.CharLimit = 0
	return ti
}

func NewChat(cfg *config.Config, registry *providers.Registry, width, height int) ChatModel {
	ta := textarea.New()
	ta.Placeholder = "Message Marlin... (Enter to send, Ctrl+J for newline)"
	ta.Focus()
	ta.CharLimit = 0
	ta.ShowLineNumbers = false
	ta.SetWidth(width - 4)
	ta.SetHeight(3)

	vp := viewport.New(width-2, max(1, height-9))

	wd := cfg.WorkDir
	if wd == "" {
		wd, _ = os.Getwd()
	}

	renderer, _ := glamour.NewTermRenderer(
		glamour.WithStylePath("dark"),
		glamour.WithWordWrap(width-6),
	)

	m := ChatModel{
		cfg:      cfg,
		registry: registry,
		workDir:  wd,
		viewport: vp,
		textarea: ta,
		keyInput: newKeyInput(),
		renderer: renderer,
		width:    width,
		height:   height,
		atBottom: true,
	}

	// Load or initiate info.json
	inf, err := infomod.Find(wd)
	if err != nil {
		// No info.json found — start setup wizard
		m.setupStep = setupAskName
		m.addSystem("Welcome to Marlin  No info.json found for this directory.")
		m.addSystem("What is this project called?")
	} else {
		m.projectInfo = inf
		// Keep cwd current
		if inf.CWD != wd {
			inf.SetCWD(wd)
		}
		m.addSystem(fmt.Sprintf("Welcome to Marlin  Project: %s", inf.Name))
		m.addSystem("Type /help for commands.")
	}

	// Load input history and create a session for this launch.
	m.historyIdx = -1
	if dir, err := config.Dir(); err == nil {
		m.marlinDir = dir
		m.inputHistory = history.LoadInput(dir)
		projectName := filepath.Base(wd)
		if m.projectInfo != nil {
			projectName = m.projectInfo.Name
		}
		m.session = history.NewSession(projectName, wd)
	}

	m.refreshViewport()
	return m
}

func (m *ChatModel) Resize(width, height int) {
	m.width = width
	m.height = height
	m.textarea.SetWidth(width - 4)
	m.keyInput.Width = width - 6
	m.renderer, _ = glamour.NewTermRenderer(
		glamour.WithStylePath("dark"),
		glamour.WithWordWrap(width-6),
	)
	m.refreshViewport()
}

func (m *ChatModel) addSystem(text string) {
	m.entries = append(m.entries, chatEntry{role: "system", content: text, time: time.Now()})
}

func (m *ChatModel) addError(text string) {
	m.entries = append(m.entries, chatEntry{role: "error", content: text, time: time.Now()})
}

func (m *ChatModel) addOutput(label, text string) {
	m.entries = append(m.entries, chatEntry{role: "output", content: fmt.Sprintf("[%s]\n%s", label, text), time: time.Now()})
}

func (m ChatModel) Init() tea.Cmd {
	return textarea.Blink
}

func (m ChatModel) Update(msg tea.Msg) (ChatModel, tea.Cmd) {
	var cmds []tea.Cmd

	switch msg := msg.(type) {
	case tea.KeyMsg:
		// Masked key input mode
		if m.inputMode == modeKeyInput {
			switch msg.Type {
			case tea.KeyEnter:
				key := strings.TrimSpace(m.keyInput.Value())
				if key == "" {
					m.inputMode = modeNormal
					m.keyInput = newKeyInput()
					m.addSystem("Key entry cancelled (empty).")
				} else {
					m.cfg.SetKey(m.keyProvider, key)
					m.cfg.Save()
					*m.registry = *providers.NewRegistry(m.cfg)
					m.inputMode = modeNormal
					m.keyInput = newKeyInput()
					m.addSystem(fmt.Sprintf("API key saved for %s.", m.keyProvider))
					m.textarea.Focus()
				}
				m.refreshViewport()
				return m, textarea.Blink
			case tea.KeyEsc:
				m.inputMode = modeNormal
				m.keyInput = newKeyInput()
				m.addSystem("Key entry cancelled.")
				m.refreshViewport()
				m.textarea.Focus()
				return m, textarea.Blink
			}
			var kiCmd tea.Cmd
			m.keyInput, kiCmd = m.keyInput.Update(msg)
			return m, kiCmd
		}

		// Cancel: works during streaming or rate-limit wait
		if msg.String() == "ctrl+c" {
			if m.rateLimited {
				m.rateLimited = false
				m.activeGoal = ""
				m.toolIterations = 0
				m.addSystem("Cancelled (rate-limit wait aborted).")
				m.refreshViewport()
				return m, nil
			}
			if m.streaming {
				if m.cancelStream != nil {
					m.cancelStream()
				}
				m.streaming = false
				if m.streamBuf != "" {
					m.history = append(m.history, providers.Message{Role: "assistant", Content: m.streamBuf})
					m.entries = append(m.entries, chatEntry{role: "assistant", content: m.streamBuf + "\n\n*[cancelled]*", time: time.Now()})
					m.streamBuf = ""
				}
				m.activeGoal = ""
				m.toolIterations = 0
				m.refreshViewport()
				return m, nil
			}
		}
		switch msg.Type {
		case tea.KeyTab:
			if len(m.suggestions) > 0 {
				if completion := tabComplete(m.textarea.Value(), m.suggestions); completion != "" {
					m.textarea.SetValue(completion)
					m.updateSuggestions()
					m.refreshViewport()
				}
			}
			return m, nil

		case tea.KeyUp:
			// Navigate input history when single-line, but only swallow the key
			// when we actually moved to a history entry. Otherwise fall through so
			// the viewport can scroll up.
			if m.inputHistory != nil && !strings.Contains(m.textarea.Value(), "\n") {
				if next := m.historyIdx + 1; next < len(m.inputHistory.Entries) {
					if m.historyIdx == -1 {
						m.historyDraft = m.textarea.Value()
					}
					m.historyIdx = next
					m.textarea.SetValue(m.inputHistory.Entries[m.historyIdx])
					m.updateSuggestions()
					m.refreshViewport()
					return m, nil
				}
				// History exhausted — fall through to viewport scroll.
			}

		case tea.KeyDown:
			// Only intercept when actively browsing history.
			if m.historyIdx >= 0 {
				m.historyIdx--
				if m.historyIdx < 0 {
					m.textarea.SetValue(m.historyDraft)
					m.historyDraft = ""
				} else {
					m.textarea.SetValue(m.inputHistory.Entries[m.historyIdx])
				}
				m.updateSuggestions()
				m.refreshViewport()
				return m, nil
			}

		case tea.KeyEnter:
			if msg.Alt {
				break
			}
			input := strings.TrimSpace(m.textarea.Value())
			if input == "" {
				return m, nil
			}
			m.textarea.Reset()
			m.suggestions = nil
			m.historyIdx = -1
			m.historyDraft = ""
			m.atBottom = true
			if m.inputHistory != nil {
				m.inputHistory.Add(input)
			}

			// Setup wizard intercepts input before normal chat
			if m.setupStep != setupDone {
				return m.handleSetupInput(input)
			}

			if strings.HasPrefix(input, "/") {
				cmd := m.handleCommand(input)
				m.refreshViewport()
				return m, cmd
			}

			displayContent := input
			if len(m.attachments) > 0 {
				var tags []string
				for _, a := range m.attachments {
					tags = append(tags, fmt.Sprintf("[%s · %d lines]", filepath.Base(a.filename), a.lines))
				}
				displayContent = strings.Join(tags, " ") + "\n" + input
			}

			historyContent := m.buildMessageContent(input)
			m.attachments = nil
			m.toolIterations = 0
			m.activeGoal = input // track goal for agentic context

			m.entries = append(m.entries, chatEntry{role: "user", content: displayContent, time: time.Now()})
			m.history = append(m.history, providers.Message{Role: "user", Content: historyContent})
			m.refreshViewport()
			return m, m.startStream()
		}

	case attachFileMsg:
		m = m.handleAttachFileMsg(msg)
		m.refreshViewport()

	case rateLimitTickMsg:
		if m.rateLimited {
			m.rateLimitSecs--
			if m.rateLimitSecs <= 0 {
				m.rateLimited = false
				m.addSystem("Rate limit cleared — resuming goal...")
				m.refreshViewport()
				return m, m.startStream()
			}
			m.refreshViewport()
			return m, tickRateLimit()
		}

	case streamChunkMsg:
		if msg.retryAfter > 0 {
			// Rate limited — pause and auto-resume
			m.streaming = false
			m.rateLimited = true
			m.rateLimitSecs = msg.retryAfter
			m.rateLimitTotal = msg.retryAfter
			m.addSystem(fmt.Sprintf("Rate limited by %s. Resuming automatically in %ds...",
				m.cfg.ActiveProvider, msg.retryAfter))
			m.refreshViewport()
			return m, tickRateLimit()
		}
		if msg.err != nil {
			m.streaming = false
			m.streamBuf = ""
			m.rateLimited = false
			m.addError(msg.err.Error())
			m.refreshViewport()
			return m, nil
		}
		if msg.done {
			m.streaming = false
			if msg.rateLimit != nil {
				m.rlState = msg.rateLimit
			}

			// Agentic tool-use loop — no hard iteration limit, goal drives termination.
			// Safety cap at 100 to prevent runaway loops on bugs.
			if len(msg.toolCalls) > 0 {
				const safetyCap = 100
				if m.toolIterations >= safetyCap {
					m.addError(fmt.Sprintf("safety cap reached (%d tool calls). Type your next message to continue.", safetyCap))
					m.streamBuf = ""
					m.activeGoal = ""
					m.refreshViewport()
					return m, nil
				}
				// Commit the assistant's text (if any) and its tool calls to history
				text := strings.TrimSpace(m.streamBuf)
				m.history = append(m.history, providers.Message{
					Role:      "assistant",
					Content:   text,
					ToolCalls: toToolCallMsgs(msg.toolCalls),
				})
				if text != "" {
					cleaned, updated := m.applyInfoUpdates(text)
					m.entries = append(m.entries, chatEntry{role: "assistant", content: cleaned, time: time.Now()})
					if updated {
						m.addSystem("info.json updated.")
					}
				}
				m.streamBuf = ""
				// Show what tools will be called
				for _, tc := range msg.toolCalls {
					m.entries = append(m.entries, chatEntry{
						role:     "tool_call",
						toolName: tc.Name,
						content:  tc.Input,
						time:     time.Now(),
					})
				}
				m.refreshViewport()

				// Execute tools asynchronously
				calls := msg.toolCalls
				workDir := m.workDir
				allowed := m.isAllowed
				marlinDir := m.marlinDir
				var cexec func(string, string) (string, error)
				if m.sandboxMode && m.sandboxManager != nil && m.sandboxManager.IsRunning() {
					cexec = m.sandboxManager.Exec
				}
				var snapFn func(string, string)
				if marlinDir != "" {
					snapFn = func(absPath, tool string) {
						snapshots.Take(marlinDir, workDir, absPath, tool)
					}
				}
				return m, func() tea.Msg {
					results := make([]toolExecResult, len(calls))
					for i, call := range calls {
						res := tools.Execute(call.Name, call.Input, workDir, allowed, cexec, snapFn)
						results[i] = toolExecResult{call: call, output: res.Output, isError: res.IsError}
					}
					return toolsExecutedMsg{results: results}
				}
			}

			// Normal response — no tool calls, goal is achieved
			if m.streamBuf != "" {
				cleaned, updated := m.applyInfoUpdates(m.streamBuf)
				m.history = append(m.history, providers.Message{Role: "assistant", Content: cleaned})
				m.entries = append(m.entries, chatEntry{role: "assistant", content: cleaned, time: time.Now()})
				if updated {
					m.addSystem("info.json updated.")
				}
				m.streamBuf = ""
			} else {
				// Model returned an empty response — surface it rather than going silent.
				m.addError("Model returned an empty response. Try rephrasing or check your API key/quota.")
			}
			if m.toolIterations > 0 {
				m.addSystem(fmt.Sprintf("Goal complete. (%d tool calls)", m.toolIterations))
			}
			m.toolIterations = 0
			m.activeGoal = ""
			m.saveSession()
			m.refreshViewport()
			return m, nil
		}
		m.streamBuf += msg.content
		m.refreshViewport()
		return m, waitForChunk(m.streamCh)

	case toolsExecutedMsg:
		for _, r := range msg.results {
			// Show result in chat
			m.entries = append(m.entries, chatEntry{
				role:     "tool_result",
				toolName: r.call.Name,
				content:  r.output,
				isError:  r.isError,
				time:     time.Now(),
			})
			// Add to history for the next AI turn
			m.history = append(m.history, providers.Message{
				Role:       "tool",
				Content:    r.output,
				ToolUseID:  r.call.ID, // Claude
				ToolCallID: r.call.ID, // OpenAI
				IsError:    r.isError,
			})
		}
		m.toolIterations++
		m.refreshViewport()
		// Continue the agentic loop
		return m, m.startStream()

	case sandboxReadyMsg:
		if msg.err != nil {
			m.addError("Sandbox failed: " + msg.err.Error())
		} else {
			m.sandboxManager = msg.mgr
			m.sandboxMode = true
			m.addSystem(fmt.Sprintf(
				"Sandbox active (%s). AI can run commands freely — writes outside the project directory are blocked.",
				msg.mgr.BackendName(),
			))
		}
		m.refreshViewport()
		return m, func() tea.Msg {
			return SetStatusMsg{
				Provider:       m.cfg.ActiveProvider,
				Model:          m.cfg.ActiveModel,
				SandboxActive:  m.sandboxMode,
				SandboxBackend: sandboxBackendName(m.sandboxManager),
			}
		}

	case cmdOutputMsg:
		if msg.err != nil {
			m.addOutput(msg.label, "error: "+msg.err.Error())
		} else {
			m.addOutput(msg.label, msg.output)
		}
		m.refreshViewport()
	}

	if m.inputMode == modeNormal {
		var taCmd tea.Cmd
		m.textarea, taCmd = m.textarea.Update(msg)
		cmds = append(cmds, taCmd)

		m.updateSuggestions()
		m.refreshViewport()

		prevOffset := m.viewport.YOffset
		var vpCmd tea.Cmd
		m.viewport, vpCmd = m.viewport.Update(msg)
		cmds = append(cmds, vpCmd)

		// Track whether user has scrolled away from the bottom.
		if m.viewport.YOffset < prevOffset {
			m.atBottom = false
		} else if m.viewport.AtBottom() {
			m.atBottom = true
		}
	}

	return m, tea.Batch(cmds...)
}

// handleSetupInput advances the setup wizard one step at a time.
func (m ChatModel) handleSetupInput(input string) (ChatModel, tea.Cmd) {
	switch m.setupStep {
	case setupAskName:
		m.setupName = input
		m.entries = append(m.entries, chatEntry{role: "user", content: input, time: time.Now()})
		m.setupStep = setupAskDesc
		m.addSystem("Brief description of what it does:")

	case setupAskDesc:
		m.setupDesc = input
		m.entries = append(m.entries, chatEntry{role: "user", content: input, time: time.Now()})
		m.setupStep = setupAskRun
		m.addSystem("Command to run the project? (press Enter to skip)")

	case setupAskRun:
		runCmd := input
		if runCmd == "" {
			runCmd = ""
		}
		m.entries = append(m.entries, chatEntry{role: "user", content: input, time: time.Now()})

		inf := infomod.New(m.workDir, m.setupName, m.setupDesc, runCmd)
		if err := inf.Save(); err != nil {
			m.addError("Failed to create info.json: " + err.Error())
		} else {
			m.projectInfo = inf
			m.addSystem(fmt.Sprintf("info.json created at %s", inf.Path()))
			m.addSystem("Type /help for commands.")
		}
		m.setupStep = setupDone
	}

	m.refreshViewport()
	return m, nil
}

// applyInfoUpdates strips MARLIN:INFO blocks from the response, applies any JSON
// patch to info.json, and returns the cleaned text plus whether an update happened.
func (m *ChatModel) applyInfoUpdates(response string) (string, bool) {
	matches := reInfoBlock.FindAllStringSubmatch(response, -1)
	if len(matches) == 0 || m.projectInfo == nil {
		return response, false
	}

	updated := false
	for _, match := range matches {
		if err := m.projectInfo.MergeJSON(match[1]); err == nil {
			m.projectInfo.Save()
			updated = true
		}
	}

	cleaned := reInfoBlock.ReplaceAllString(response, "")
	cleaned = strings.TrimSpace(cleaned)
	return cleaned, updated
}

func (m ChatModel) View() string {
	var inputArea string

	if m.inputMode == modeKeyInput {
		label := StyleUserLabel.Render(fmt.Sprintf(" API key for %s ", m.keyProvider))
		inputArea = StyleInputFocused.Width(m.width - 4).Render(
			lipgloss.JoinVertical(lipgloss.Left, label, m.keyInput.View()),
		)
	} else {
		border := StyleInputBorder
		if !m.streaming {
			border = StyleInputFocused
		}
		inputArea = border.Width(m.width - 4).Render(m.textarea.View())
	}

	var hintParts []string
	switch {
	case m.rateLimited:
		pct := 1.0
		if m.rateLimitTotal > 0 {
			pct = float64(m.rateLimitSecs) / float64(m.rateLimitTotal)
		}
		bar := rateBar(pct, 20)
		hintParts = append(hintParts, StyleErrorMsg.Render(fmt.Sprintf("rate limited · resuming in %ds  %s", m.rateLimitSecs, bar)))

	case m.inputMode == modeKeyInput:
		hintParts = append(hintParts, "Enter:confirm  Esc:cancel")

	case m.setupStep != setupDone:
		hintParts = append(hintParts, "Enter:confirm  (Ctrl+J for multi-line)")

	case m.activeGoal != "":
		goal := m.activeGoal
		if len(goal) > 50 {
			goal = goal[:47] + "..."
		}
		hintParts = append(hintParts, StyleSuccessMsg.Render(fmt.Sprintf("goal: %s", goal)))
		hintParts = append(hintParts, StyleSystemMsg.Render(fmt.Sprintf("%d tool calls  ·  Ctrl+C to cancel", m.toolIterations)))

	default:
		hintParts = append(hintParts, "Enter:send  Ctrl+J:newline  Ctrl+C:cancel  /help:commands")
		if m.sandboxMode {
			bak := sandboxBackendName(m.sandboxManager)
			hintParts = append(hintParts, StyleSuccessMsg.Render("sandbox:"+bak))
		}
		if len(m.attachments) > 0 {
			var names []string
			for _, a := range m.attachments {
				names = append(names, fmt.Sprintf("%s(%d)", filepath.Base(a.filename), a.lines))
			}
			hintParts = append(hintParts, StyleSuccessMsg.Render("attached: "+strings.Join(names, ", ")))
		}
	}

	hint := StyleSystemMsg.Render("  " + strings.Join(hintParts, "  ·  "))

	parts := []string{m.viewport.View()}
	if len(m.suggestions) > 0 {
		parts = append(parts, m.renderSuggestions())
	}
	parts = append(parts, inputArea, hint)

	return lipgloss.JoinVertical(lipgloss.Left, parts...)
}

func (m *ChatModel) buildMessageContent(userText string) string {
	if len(m.attachments) == 0 {
		return userText
	}
	var sb strings.Builder
	for _, a := range m.attachments {
		ext := strings.TrimPrefix(filepath.Ext(a.filename), ".")
		sb.WriteString(fmt.Sprintf("File: %s\n```%s\n%s\n```\n\n", a.filename, ext, a.content))
	}
	sb.WriteString(userText)
	return sb.String()
}

// effectiveSystemPrompt combines the project info.json context with any user-set system prompt.
func (m *ChatModel) effectiveSystemPrompt() string {
	var sb strings.Builder
	sb.WriteString("You are Marlin, an AI coding assistant running in a terminal.\n")
	sb.WriteString("You help the user write, debug, and understand code.\n\n")

	sb.WriteString("## Tools\n")
	sb.WriteString("You have the following tools and MUST use them to act directly — never tell the user to do something manually that a tool can do:\n")
	sb.WriteString("- read_file: read any file\n")
	sb.WriteString("- write_file: create or overwrite a file (use this to create READMEs, configs, source files, etc.)\n")
	sb.WriteString("- edit_file: replace a specific string in a file (preferred for targeted edits)\n")
	sb.WriteString("- run_command: run a shell command\n")
	sb.WriteString("- list_directory: list files in a directory\n")
	sb.WriteString("- create_directory: create a directory\n\n")
	sb.WriteString("When asked to create a file, write code, edit something, or run a command — DO IT with the appropriate tool. ")
	sb.WriteString("Do not explain how the user could do it themselves. Do not ask for confirmation before using tools. Just act.\n\n")

	if m.projectInfo != nil {
		sb.WriteString("## Project context (info.json)\n```json\n")
		sb.WriteString(m.projectInfo.ForSystemPrompt())
		sb.WriteString("\n```\n\n")
	} else {
		sb.WriteString(fmt.Sprintf("Working directory: %s\n\n", m.workDir))
	}

	sb.WriteString("You may update the project info.json at any time by including a block in your response:\n")
	sb.WriteString("<!--MARLIN:INFO {\"field\": \"value\"} -->\n")
	sb.WriteString("Only include fields you want to change. This block will not be shown to the user.\n")

	if m.sandboxMode {
		sb.WriteString("\n## Sandbox Mode\n")
		sb.WriteString("You are running inside a secure sandbox. You may call run_command freely without restriction — ")
		sb.WriteString("commands are executed in an isolated environment that cannot modify files outside the project directory.\n")
	}

	if m.activeGoal != "" {
		sb.WriteString("\n## Active Goal\n")
		sb.WriteString(m.activeGoal + "\n")
		sb.WriteString("\nWork toward this goal using tools. Keep calling tools until the task is fully complete.\n")
		sb.WriteString("Only produce a plain text response (with no tool calls) when the goal is achieved or you need user input.\n")
		sb.WriteString(fmt.Sprintf("Progress so far: %d tool calls made.\n", m.toolIterations))
	}

	if m.cfg.SystemPrompt != "" {
		sb.WriteString("\nAdditional instructions:\n")
		sb.WriteString(m.cfg.SystemPrompt)
	}

	return sb.String()
}

func (m *ChatModel) refreshViewport() {
	// Recalculate viewport height, shrinking to make room for the suggestion panel.
	suggestH := 0
	if len(m.suggestions) > 0 {
		shown := len(m.suggestions)
		if shown > 8 {
			shown = 8
		}
		suggestH = shown + 2 // content rows + rounded border top/bottom
	}
	const inputH = 5
	const hintH = 1
	vpH := m.height - inputH - hintH - suggestH
	if vpH < 1 {
		vpH = 1
	}
	m.viewport.Width = m.width - 2
	m.viewport.Height = vpH

	var sb strings.Builder

	for _, e := range m.entries {
		sb.WriteString(m.renderEntry(e))
		sb.WriteString("\n")
	}

	if m.streaming && m.streamBuf != "" {
		sb.WriteString(StyleAssistantLabel.Render("Marlin") + "\n")
		display := reInfoBlock.ReplaceAllString(m.streamBuf, "")
		sb.WriteString(display + StyleCursor.Render(" ") + "\n\n")
	}
	if m.streaming && m.toolIterations > 0 {
		sb.WriteString(StyleSystemMsg.Render(fmt.Sprintf("  ⚙ thinking (tool loop %d/10)…", m.toolIterations)) + "\n")
	}

	m.viewport.SetContent(sb.String())
	if m.atBottom {
		m.viewport.GotoBottom()
	}
}

func (m *ChatModel) renderEntry(e chatEntry) string {
	switch e.role {
	case "user":
		ts := StyleSystemMsg.Render(e.time.Format("15:04"))
		label := StyleUserLabel.Render("You") + " " + ts
		return label + "\n" + e.content + "\n"

	case "assistant":
		ts := StyleSystemMsg.Render(e.time.Format("15:04"))
		label := StyleAssistantLabel.Render("Marlin") + " " + ts
		rendered, err := m.renderer.Render(e.content)
		if err != nil {
			rendered = e.content
		}
		return label + "\n" + strings.TrimRight(rendered, "\n") + "\n"

	case "system":
		return StyleSystemMsg.Render("  ‣ "+e.content) + "\n"

	case "error":
		return StyleErrorMsg.Render("  ✗ "+e.content) + "\n"

	case "output":
		lines := strings.Split(e.content, "\n")
		var out strings.Builder
		out.WriteString(StyleSuccessMsg.Render("  ▶ "+lines[0]) + "\n")
		if len(lines) > 1 {
			body := strings.Join(lines[1:], "\n")
			rendered, err := m.renderer.Render("```\n" + body + "\n```")
			if err != nil {
				out.WriteString(body)
			} else {
				out.WriteString(rendered)
			}
		}
		return out.String()

	case "tool_call":
		label := StyleHelpKey.Render("  ⚙ " + e.toolName)
		// Pretty-print the JSON input on one line if it fits, else compact
		input := e.content
		if len(input) > 120 {
			input = input[:117] + "..."
		}
		return label + "  " + StyleSystemMsg.Render(input) + "\n"

	case "tool_result":
		var icon string
		var style lipgloss.Style
		if e.isError {
			icon = "  ✗ "
			style = StyleErrorMsg
		} else {
			icon = "  ✓ "
			style = StyleSuccessMsg
		}
		// Show up to 8 lines of output; truncate the rest
		lines := strings.Split(strings.TrimRight(e.content, "\n"), "\n")
		const maxLines = 8
		var shown []string
		if len(lines) > maxLines {
			shown = append(lines[:maxLines], fmt.Sprintf("… (%d more lines)", len(lines)-maxLines))
		} else {
			shown = lines
		}
		return style.Render(icon+e.toolName) + "\n" + StyleSystemMsg.Render("    "+strings.Join(shown, "\n    ")) + "\n"
	}
	return ""
}

func waitForChunk(ch <-chan providers.StreamChunk) tea.Cmd {
	return func() tea.Msg {
		c := <-ch
		return streamChunkMsg{
			content:    c.Content,
			done:       c.Done,
			err:        c.Error,
			toolCalls:  c.ToolCalls,
			retryAfter: c.RetryAfter,
			rateLimit:  c.RateLimit,
		}
	}
}

func tickRateLimit() tea.Cmd {
	return tea.Tick(time.Second, func(_ time.Time) tea.Msg { return rateLimitTickMsg{} })
}

// estimateTokens returns a rough token count for the current history + system prompt.
// Uses the 4-chars-per-token heuristic — good enough for a conservative budget check.
func (m *ChatModel) estimateTokens() int {
	n := len(m.effectiveSystemPrompt()) / 4
	for _, msg := range m.history {
		n += len(msg.Content) / 4
		for _, tc := range msg.ToolCalls {
			n += len(tc.Input) / 4
		}
	}
	return n
}

func (m *ChatModel) startStream() tea.Cmd {
	// Proactive rate-limit check: if the provider told us how much budget remains,
	// compare against our estimated payload size before making the round-trip.
	if rl := m.rlState; rl != nil {
		estimate := m.estimateTokens()
		waitSecs := 0

		if rl.RemainingTokens >= 0 && estimate > rl.RemainingTokens && !rl.ResetTokensAt.IsZero() {
			waitSecs = int(time.Until(rl.ResetTokensAt).Seconds()) + 1
		}
		if rl.RemainingRequests == 0 && !rl.ResetRequestsAt.IsZero() {
			if s := int(time.Until(rl.ResetRequestsAt).Seconds()) + 1; s > waitSecs {
				waitSecs = s
			}
		}

		if waitSecs > 0 {
			m.rateLimited = true
			m.rateLimitSecs = waitSecs
			m.rateLimitTotal = waitSecs
			m.rlState = nil // clear so we don't re-trigger on the next check
			m.addSystem(fmt.Sprintf(
				"Proactive pause: ~%d tokens estimated, %d remaining (window resets in %ds).",
				estimate, rl.RemainingTokens, waitSecs,
			))
			m.refreshViewport()
			return tickRateLimit()
		}
	}

	ctx, cancel := context.WithCancel(context.Background())
	m.cancelStream = cancel
	m.streaming = true
	m.streamBuf = ""

	provider, err := m.registry.Get(m.cfg.ActiveProvider)
	if err != nil {
		m.streaming = false
		m.addError(err.Error())
		return nil
	}

	toolDefs := toProviderToolDefs(tools.All())
	ch, err := provider.Stream(ctx, m.cfg.ActiveModel, m.history, m.effectiveSystemPrompt(), m.cfg.MaxTokens, toolDefs)
	if err != nil {
		m.streaming = false
		cancel()
		m.addError(err.Error())
		return nil
	}

	m.streamCh = ch
	return waitForChunk(ch)
}

// toProviderToolDefs converts tools.Def → providers.ToolDef.
func toProviderToolDefs(defs []tools.Def) []providers.ToolDef {
	out := make([]providers.ToolDef, len(defs))
	for i, d := range defs {
		props := make(map[string]providers.ToolProp, len(d.Properties))
		for k, p := range d.Properties {
			props[k] = providers.ToolProp{Type: p.Type, Description: p.Description}
		}
		out[i] = providers.ToolDef{
			Name:        d.Name,
			Description: d.Description,
			Properties:  props,
			Required:    d.Required,
		}
	}
	return out
}

// toToolCallMsgs converts []providers.ToolCall → []providers.ToolCallMsg for history.
func toToolCallMsgs(calls []providers.ToolCall) []providers.ToolCallMsg {
	out := make([]providers.ToolCallMsg, len(calls))
	for i, c := range calls {
		out[i] = providers.ToolCallMsg{ID: c.ID, Name: c.Name, Input: c.Input}
	}
	return out
}

func (m *ChatModel) handleCommand(input string) tea.Cmd {
	parts := strings.Fields(input)
	if len(parts) == 0 {
		return nil
	}
	cmd := strings.ToLower(parts[0])
	args := parts[1:]

	switch cmd {
	case "/help":
		m.addSystem(helpText())

	case "/clear":
		m.entries = nil
		m.history = nil
		m.attachments = nil
		m.addSystem("Chat cleared.")

	case "/provider", "/p":
		if len(args) == 0 {
			m.addSystem("Usage: /provider <name>  —  available: " + strings.Join(m.registry.Names(), ", "))
			return nil
		}
		name := strings.ToLower(args[0])
		_, err := m.registry.Get(name)
		if err != nil {
			m.addError(err.Error())
			return nil
		}
		m.cfg.ActiveProvider = name
		pcfg := m.cfg.Providers[name]
		m.cfg.ActiveModel = pcfg.Model
		m.cfg.Save()
		m.addSystem(fmt.Sprintf("Switched to provider: %s  model: %s", name, m.cfg.ActiveModel))
		return func() tea.Msg {
			return SetStatusMsg{Provider: m.cfg.ActiveProvider, Model: m.cfg.ActiveModel}
		}

	case "/model", "/m":
		if len(args) == 0 {
			p, err := m.registry.Get(m.cfg.ActiveProvider)
			if err == nil {
				m.addSystem("Available models: " + strings.Join(p.Models(), ", "))
			}
			return nil
		}
		model := args[0]
		m.cfg.ActiveModel = model
		pcfg := m.cfg.Providers[m.cfg.ActiveProvider]
		pcfg.Model = model
		m.cfg.Providers[m.cfg.ActiveProvider] = pcfg
		m.cfg.Save()
		m.addSystem(fmt.Sprintf("Model set to: %s", model))
		return func() tea.Msg {
			return SetStatusMsg{Provider: m.cfg.ActiveProvider, Model: m.cfg.ActiveModel}
		}

	case "/key":
		if len(args) == 0 {
			m.addSystem("Usage: /key <provider>  (masked prompt)")
			return nil
		}
		provider := strings.ToLower(args[0])
		if _, err := m.registry.Get(provider); err != nil {
			m.addError(err.Error())
			return nil
		}
		if len(args) >= 2 {
			m.cfg.SetKey(provider, args[1])
			m.cfg.Save()
			*m.registry = *providers.NewRegistry(m.cfg)
			m.addSystem(fmt.Sprintf("API key saved for %s. (tip: /key %s alone uses a masked prompt)", provider, provider))
			return nil
		}
		m.keyProvider = provider
		m.inputMode = modeKeyInput
		m.keyInput = newKeyInput()
		m.keyInput.Width = m.width - 6
		return m.keyInput.Focus()

	case "/endpoint":
		if len(args) < 2 {
			m.addSystem("Usage: /endpoint <provider> <url>")
			return nil
		}
		m.cfg.SetEndpoint(args[0], args[1])
		m.cfg.Save()
		*m.registry = *providers.NewRegistry(m.cfg)
		m.addSystem(fmt.Sprintf("Endpoint updated for %s: %s", args[0], args[1]))

	case "/system", "/sys":
		if len(args) == 0 {
			if m.cfg.SystemPrompt == "" {
				m.addSystem("No custom system prompt. Use /system <text> to set one.")
			} else {
				m.addSystem("Custom system prompt: " + m.cfg.SystemPrompt)
			}
			return nil
		}
		m.cfg.SystemPrompt = strings.Join(args, " ")
		m.cfg.Save()
		m.addSystem("System prompt updated.")

	case "/info":
		if len(args) == 0 {
			if m.projectInfo == nil {
				m.addSystem("No info.json loaded. Use /init to create one.")
				return nil
			}
			m.addSystem("info.json:\n```json\n" + m.projectInfo.ForSystemPrompt() + "\n```")
			return nil
		}
		sub := strings.ToLower(args[0])
		switch sub {
		case "set":
			// /info set <json-object>
			if len(args) < 2 {
				m.addSystem("Usage: /info set {\"field\": \"value\"}")
				return nil
			}
			if m.projectInfo == nil {
				m.addError("No info.json loaded.")
				return nil
			}
			raw := strings.Join(args[1:], " ")
			if err := m.projectInfo.MergeJSON(raw); err != nil {
				m.addError("JSON error: " + err.Error())
				return nil
			}
			if err := m.projectInfo.Save(); err != nil {
				m.addError("Save error: " + err.Error())
				return nil
			}
			m.addSystem("info.json updated.")
		case "notes":
			if len(args) < 2 {
				m.addSystem("Usage: /info notes <text>")
				return nil
			}
			if m.projectInfo == nil {
				m.addError("No info.json loaded.")
				return nil
			}
			m.projectInfo.Notes = strings.Join(args[1:], " ")
			if err := m.projectInfo.Save(); err != nil {
				m.addError("Save error: " + err.Error())
				return nil
			}
			m.addSystem("Notes saved.")
		case "edit":
			if m.projectInfo == nil {
				m.addError("No info.json loaded.")
				return nil
			}
			return func() tea.Msg { return OpenEditorMsg{Filename: m.projectInfo.Path(), Readonly: false} }
		default:
			m.addSystem("Usage: /info  /info set {json}  /info notes <text>  /info edit")
		}

	case "/attach", "/a":
		if len(args) == 0 {
			if len(m.attachments) == 0 {
				m.addSystem("No files attached. Usage: /attach <file>")
			} else {
				var names []string
				for _, a := range m.attachments {
					names = append(names, fmt.Sprintf("%s (%d lines)", a.filename, a.lines))
				}
				m.addSystem("Attached:\n" + strings.Join(names, "\n"))
			}
			return nil
		}
		filename := m.resolvePath(args[0])
		return func() tea.Msg {
			data, err := os.ReadFile(filename)
			if err != nil {
				return cmdOutputMsg{label: "attach", output: "", err: err}
			}
			content := string(data)
			lineCount := strings.Count(content, "\n") + 1
			return attachFileMsg{filename: filename, content: content, lines: lineCount}
		}

	case "/detach":
		if len(args) == 0 {
			m.attachments = nil
			m.addSystem("All attachments cleared.")
			return nil
		}
		name := args[0]
		newAtt := m.attachments[:0]
		removed := false
		for _, a := range m.attachments {
			if filepath.Base(a.filename) == name || a.filename == name {
				removed = true
				continue
			}
			newAtt = append(newAtt, a)
		}
		m.attachments = newAtt
		if removed {
			m.addSystem(fmt.Sprintf("Detached: %s", name))
		} else {
			m.addError(fmt.Sprintf("No attachment named %q", name))
		}

	case "/run":
		cmdName := "run"
		if len(args) > 0 {
			cmdName = args[0]
		}
		inf, err := infomod.Find(m.workDir)
		if err != nil {
			m.addError(err.Error())
			return nil
		}
		label := fmt.Sprintf("%s: %s", inf.Dir(), cmdName)
		m.addSystem(fmt.Sprintf("Running '%s'...", cmdName))
		return func() tea.Msg {
			out, err := inf.Execute(cmdName)
			return cmdOutputMsg{label: label, output: out, err: err}
		}

	case "/exec":
		if len(args) == 0 {
			m.addSystem("Usage: /exec <shell command>")
			return nil
		}
		shellCmd := strings.Join(args, " ")
		if !m.isAllowed(shellCmd) {
			m.addError(fmt.Sprintf("Command not allowed: %q\nUse /allow <prefix> to permit it.", shellCmd))
			return nil
		}
		m.addSystem(fmt.Sprintf("Running: %s", shellCmd))
		wd := m.workDir
		return func() tea.Msg {
			c := exec.Command("sh", "-c", shellCmd)
			c.Dir = wd
			out, err := c.CombinedOutput()
			return cmdOutputMsg{label: "exec", output: strings.TrimSpace(string(out)), err: err}
		}

	case "/allow":
		if len(args) == 0 {
			if len(m.cfg.AllowedCommands) == 0 {
				m.addSystem("No commands allowed. Use /allow <prefix> to permit.")
			} else {
				m.addSystem("Allowed prefixes: " + strings.Join(m.cfg.AllowedCommands, ", "))
			}
			return nil
		}
		pattern := strings.Join(args, " ")
		m.cfg.AllowedCommands = append(m.cfg.AllowedCommands, pattern)
		m.cfg.Save()
		m.addSystem(fmt.Sprintf("Allowed: %q", pattern))

	case "/edit":
		if len(args) == 0 {
			m.addSystem("Usage: /edit <filename>")
			return nil
		}
		filename := m.resolvePath(args[0])
		return func() tea.Msg { return OpenEditorMsg{Filename: filename, Readonly: false} }

	case "/view":
		dir := m.workDir
		if len(args) > 0 {
			p := m.resolvePath(args[0])
			fi, err := os.Stat(p)
			if err != nil {
				m.addError("Not found: " + args[0])
				return nil
			}
			if fi.IsDir() {
				dir = p
			} else {
				dir = filepath.Dir(p)
			}
		}
		d := dir
		return func() tea.Msg { return OpenFileTreeMsg{Dir: d} }

	case "/open":
		if len(args) == 0 {
			m.addSystem("Usage: /open <filename>")
			return nil
		}
		filename := m.resolvePath(args[0])
		return func() tea.Msg { return OpenEditorMsg{Filename: filename, Readonly: true} }

	case "/cat":
		if len(args) == 0 {
			m.addSystem("Usage: /cat <filename>")
			return nil
		}
		filename := m.resolvePath(args[0])
		return func() tea.Msg {
			data, err := os.ReadFile(filename)
			return cmdOutputMsg{label: filename, output: string(data), err: err}
		}

	case "/ls":
		dir := m.workDir
		if len(args) > 0 {
			dir = m.resolvePath(args[0])
		}
		return func() tea.Msg {
			entries, err := os.ReadDir(dir)
			if err != nil {
				return cmdOutputMsg{label: "ls", output: "", err: err}
			}
			var lines []string
			for _, e := range entries {
				if e.IsDir() {
					lines = append(lines, e.Name()+"/")
				} else {
					lines = append(lines, e.Name())
				}
			}
			return cmdOutputMsg{label: dir, output: strings.Join(lines, "\n")}
		}

	case "/cd":
		if len(args) == 0 {
			m.addSystem("Current directory: " + m.workDir)
			return nil
		}
		newDir := m.resolvePath(args[0])
		fi, err := os.Stat(newDir)
		if err != nil || !fi.IsDir() {
			m.addError(fmt.Sprintf("Not a directory: %s", newDir))
			return nil
		}
		m.workDir = newDir
		m.cfg.WorkDir = newDir
		m.cfg.Save()
		// Sync cwd into info.json
		if m.projectInfo != nil {
			m.projectInfo.SetCWD(newDir)
		}
		m.addSystem("Directory: " + newDir)

	case "/pwd":
		m.addSystem("Directory: " + m.workDir)

	case "/init":
		// Shortcut to re-run setup wizard
		m.setupStep = setupAskName
		m.addSystem("Creating new info.json. What is this project called?")

	case "/providers":
		names := m.registry.Names()
		var lines []string
		for _, n := range names {
			p, _ := m.registry.Get(n)
			mark := "  "
			if n == m.cfg.ActiveProvider {
				mark = "▶ "
			}
			lines = append(lines, fmt.Sprintf("%s%s  [%s]", mark, n, strings.Join(p.Models()[:min(2, len(p.Models()))], ", ")+"..."))
		}
		m.addSystem("Providers:\n" + strings.Join(lines, "\n"))

	case "/models":
		p, err := m.registry.Get(m.cfg.ActiveProvider)
		if err != nil {
			m.addError(err.Error())
			return nil
		}
		m.addSystem("Models for " + m.cfg.ActiveProvider + ":\n" + strings.Join(p.Models(), "\n"))

	case "/tokens":
		if len(args) > 0 {
			n := 0
			fmt.Sscan(args[0], &n)
			if n > 0 {
				m.cfg.MaxTokens = n
				m.cfg.Save()
				m.addSystem(fmt.Sprintf("Max tokens: %d", n))
			}
		} else {
			m.addSystem(fmt.Sprintf("Max tokens: %d  (use /tokens <n> to change)", m.cfg.MaxTokens))
		}

	case "/sandbox":
		sub := ""
		if len(args) > 0 {
			sub = strings.ToLower(args[0])
		}
		switch sub {
		case "", "status":
			if m.sandboxMode && m.sandboxManager != nil {
				m.addSystem(fmt.Sprintf("Sandbox: ACTIVE  backend:%s  — AI run_command calls execute in isolation.",
					m.sandboxManager.BackendName()))
			} else {
				avail := container.Detect()
				if avail == "none" {
					m.addSystem("Sandbox: OFF  (no backend available — install Docker or use macOS)")
				} else {
					m.addSystem(fmt.Sprintf("Sandbox: OFF  available backend: %s  — use /sandbox on to enable", avail))
				}
			}

		case "on":
			if m.sandboxMode {
				m.addSystem(fmt.Sprintf("Sandbox already active (%s).", sandboxBackendName(m.sandboxManager)))
				return nil
			}
			mgr := container.New()
			if mgr == nil {
				m.addError("No sandbox backend available. Install Docker or run on macOS for sandbox-exec.")
				return nil
			}
			m.addSystem(fmt.Sprintf("Starting sandbox (%s)...", mgr.BackendName()))
			wd := m.workDir
			return func() tea.Msg {
				if err := mgr.Start(wd); err != nil {
					return sandboxReadyMsg{err: err}
				}
				return sandboxReadyMsg{mgr: mgr}
			}

		case "off":
			if !m.sandboxMode {
				m.addSystem("Sandbox is already off.")
				return nil
			}
			if m.sandboxManager != nil {
				m.sandboxManager.Stop()
				m.sandboxManager = nil
			}
			m.sandboxMode = false
			m.addSystem("Sandbox stopped. Commands require /allow again.")
			return func() tea.Msg {
				return SetStatusMsg{
					Provider:      m.cfg.ActiveProvider,
					Model:         m.cfg.ActiveModel,
					SandboxActive: false,
				}
			}

		default:
			m.addSystem("Usage: /sandbox [on|off|status]")
		}

	case "/revert":
		if len(args) == 0 {
			m.addSystem("Usage: /revert <file> [n]  —  list snapshots or restore one")
			return nil
		}
		absPath := m.resolvePath(args[0])
		snaps := snapshots.List(m.marlinDir, m.workDir, absPath)
		if len(snaps) == 0 {
			m.addSystem(fmt.Sprintf("No snapshots for %s — Marlin snapshots files before every AI edit.", args[0]))
			return nil
		}
		if len(args) < 2 {
			var lines []string
			for i, s := range snaps {
				lines = append(lines, fmt.Sprintf("  %2d.  %s  [%s]  %s",
					i+1,
					s.Timestamp.Format("Jan 02 15:04:05"),
					s.Tool,
					snapshots.HumanSize(s.Size),
				))
			}
			m.addSystem(fmt.Sprintf("Snapshots for %s (newest first):\n%s\n\nUse /revert %s <n> to restore.", args[0], strings.Join(lines, "\n"), args[0]))
			return nil
		}
		n := 0
		fmt.Sscan(args[1], &n)
		if n < 1 || n > len(snaps) {
			m.addError(fmt.Sprintf("Invalid snapshot number (1–%d).", len(snaps)))
			return nil
		}
		snap := snaps[n-1]
		if err := snapshots.Restore(m.marlinDir, m.workDir, absPath, snap.ID); err != nil {
			m.addError("Restore failed: " + err.Error())
			return nil
		}
		m.addSystem(fmt.Sprintf("Restored %s → snapshot from %s (%s, %s).",
			args[0], snap.Timestamp.Format("Jan 02 15:04:05"), snap.Tool, snapshots.HumanSize(snap.Size)))

	case "/resume":
		sessions, err := history.ListSessions(m.marlinDir)
		if err != nil || len(sessions) == 0 {
			m.addSystem("No saved sessions to resume.")
			return nil
		}
		s := sessions[0]
		m.entries = nil
		for _, e := range s.Entries {
			m.entries = append(m.entries, chatEntry{
				role: e.Role, content: e.Content, time: e.Time,
				toolName: e.ToolName, isError: e.IsError,
			})
		}
		m.history = nil
		for _, msg := range s.Messages {
			var tcs []providers.ToolCallMsg
			for _, tc := range msg.ToolCalls {
				tcs = append(tcs, providers.ToolCallMsg{ID: tc.ID, Name: tc.Name, Input: tc.Input})
			}
			m.history = append(m.history, providers.Message{
				Role:       msg.Role,
				Content:    msg.Content,
				ToolCalls:  tcs,
				ToolCallID: msg.ToolCallID,
				ToolUseID:  msg.ToolUseID,
				IsError:    msg.IsError,
			})
		}
		m.addSystem(fmt.Sprintf("Resumed: %s", s.Summary()))
		return nil

	case "/history":
		if len(args) > 0 && args[0] == "clear" {
			if err := history.ClearSessions(m.marlinDir); err != nil {
				m.addError("Failed to clear sessions: " + err.Error())
			} else {
				m.addSystem("Session history cleared.")
			}
			return nil
		}
		sessions, err := history.ListSessions(m.marlinDir)
		if err != nil || len(sessions) == 0 {
			m.addSystem("No saved sessions.")
			return nil
		}
		if len(args) > 0 {
			n := 0
			fmt.Sscan(args[0], &n)
			if n < 1 || n > len(sessions) {
				m.addError(fmt.Sprintf("Invalid session number (1–%d).", len(sessions)))
				return nil
			}
			s := sessions[n-1]
			m.entries = nil
			for _, e := range s.Entries {
				m.entries = append(m.entries, chatEntry{
					role: e.Role, content: e.Content, time: e.Time,
					toolName: e.ToolName, isError: e.IsError,
				})
			}
			m.history = nil
			for _, msg := range s.Messages {
				var tcs []providers.ToolCallMsg
				for _, tc := range msg.ToolCalls {
					tcs = append(tcs, providers.ToolCallMsg{ID: tc.ID, Name: tc.Name, Input: tc.Input})
				}
				m.history = append(m.history, providers.Message{
					Role:       msg.Role,
					Content:    msg.Content,
					ToolCalls:  tcs,
					ToolCallID: msg.ToolCallID,
					ToolUseID:  msg.ToolUseID,
					IsError:    msg.IsError,
				})
			}
			m.addSystem(fmt.Sprintf("Loaded: %s", s.Summary()))
			return nil
		}
		// List sessions (up to 20)
		limit := len(sessions)
		if limit > 20 {
			limit = 20
		}
		var lines []string
		for i, s := range sessions[:limit] {
			lines = append(lines, fmt.Sprintf("  %2d.  %s  [%s]", i+1, s.Summary(), s.Project))
		}
		m.addSystem("Saved sessions (newest first):\n" + strings.Join(lines, "\n") + "\n\nUse /history <n> to load, /history clear to delete all.")

	default:
		m.addError(fmt.Sprintf("Unknown command: %s  (type /help for list)", cmd))
	}

	return nil
}

func (m ChatModel) handleAttachFileMsg(msg attachFileMsg) ChatModel {
	for i, a := range m.attachments {
		if a.filename == msg.filename {
			m.attachments[i] = attachment{filename: msg.filename, content: msg.content, lines: msg.lines}
			m.addSystem(fmt.Sprintf("Re-attached: %s (%d lines)", filepath.Base(msg.filename), msg.lines))
			return m
		}
	}
	m.attachments = append(m.attachments, attachment{filename: msg.filename, content: msg.content, lines: msg.lines})
	m.addSystem(fmt.Sprintf("Attached: %s (%d lines)  —  send your next message to include it", filepath.Base(msg.filename), msg.lines))
	return m
}

func (m *ChatModel) isAllowed(cmd string) bool {
	for _, pattern := range m.cfg.AllowedCommands {
		if pattern == "*" || strings.HasPrefix(cmd, pattern) {
			return true
		}
	}
	return false
}

func (m *ChatModel) resolvePath(p string) string {
	if strings.HasPrefix(p, "~/") || p == "~" {
		if home, err := os.UserHomeDir(); err == nil {
			if p == "~" {
				return home
			}
			return filepath.Join(home, p[2:])
		}
	}
	if filepath.IsAbs(p) {
		return p
	}
	return filepath.Join(m.workDir, p)
}

func helpText() string {
	cmds := [][2]string{
		{"/help", "show this help"},
		{"/clear", "clear chat history and attachments"},
		{"/provider <name>", "switch provider (claude/gemini/ollama/fireworks/moonshot/custom)"},
		{"/model <name>", "switch model"},
		{"/providers", "list all providers and models"},
		{"/models", "list models for current provider"},
		{"/key <provider>", "set API key (masked — never hits shell history)"},
		{"/endpoint <provider> <url>", "set API endpoint for a provider"},
		{"/system <prompt>", "set additional system prompt"},
		{"/tokens <n>", "set max output tokens"},
		{"/info", "view info.json"},
		{"/info set {json}", "merge JSON into info.json"},
		{"/info notes <text>", "update notes in info.json"},
		{"/info edit", "open info.json in the editor"},
		{"/init", "create/re-create info.json via wizard"},
		{"/attach <file>", "attach a file to your next message"},
		{"/detach [file]", "remove attachment(s)"},
		{"/run [cmd]", "run project command from info.json (default: run)"},
		{"/exec <cmd>", "run a shell command (must be /allow-ed first)"},
		{"/allow <prefix>", "allow a shell command prefix (e.g. /allow npm)"},
		{"/sandbox [on|off|status]", "toggle isolated sandbox mode (AI runs commands freely, safely)"},
		{"/revert <file> [n]", "list file snapshots or restore one (AI snapshots before every edit)"},
		{"/resume", "resume the most recent saved session"},
		{"/history [n|clear]", "list saved sessions, load one by number, or clear all"},
		{"/edit <file>", "open file in editor"},
		{"/view [dir]", "browse file tree — arrow keys navigate, Enter opens, ← goes back"},
		{"/open <file>", "open file read-only in editor"},
		{"/cat <file>", "print file contents"},
		{"/ls [dir]", "list directory"},
		{"/cd <dir>", "change working directory (syncs info.json)"},
		{"/pwd", "show working directory"},
	}

	var sb strings.Builder
	sb.WriteString("Commands:\n")
	for _, c := range cmds {
		pad := 30 - utf8.RuneCountInString(c[0])
		if pad < 1 {
			pad = 1
		}
		sb.WriteString("  " + c[0] + strings.Repeat(" ", pad) + c[1] + "\n")
	}
	return sb.String()
}

func min(a, b int) int {
	if a < b {
		return a
	}
	return b
}

// updateSuggestions filters allCommandDefs against whatever is typed in the textarea.
// It runs on every keystroke while the input starts with "/" and has no space yet.
func (m *ChatModel) updateSuggestions() {
	val := m.textarea.Value()
	// Only active while typing the command word (no spaces, no newlines, starts with /)
	if !strings.HasPrefix(val, "/") || strings.ContainsAny(val, " \n") {
		m.suggestions = nil
		return
	}
	var matches []cmdSuggestion
	for _, def := range allCommandDefs {
		if strings.HasPrefix(def.cmd, val) {
			matches = append(matches, cmdSuggestion{
				cmdDef: def,
				exact:  def.cmd == val,
			})
		}
	}
	m.suggestions = matches
}

// renderSuggestions draws the autocomplete panel (up to 8 rows).
func (m *ChatModel) renderSuggestions() string {
	shown := m.suggestions
	if len(shown) > 8 {
		shown = shown[:8]
	}

	var lines []string
	for _, s := range shown {
		// Command: bold+bright when exactly matched, normal blue otherwise
		var cmdPart string
		if s.exact {
			cmdPart = lipgloss.NewStyle().Foreground(colAqua).Bold(true).Render(s.cmd)
		} else {
			cmdPart = StyleHelpKey.Render(s.cmd)
		}

		// Pad based on raw (unstyled) length so descriptions align
		rawLen := len(s.cmd)
		if s.args != "" {
			rawLen += 2 + len(s.args)
		}
		pad := 32 - rawLen
		if pad < 2 {
			pad = 2
		}

		argPart := ""
		if s.args != "" {
			argPart = "  " + StyleSystemMsg.Render(s.args)
		}
		descPart := StyleSystemMsg.Render(s.desc)

		lines = append(lines, "  "+cmdPart+argPart+strings.Repeat(" ", pad)+descPart)
	}

	return lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(colCobalt).
		Width(m.width - 6).
		Render(strings.Join(lines, "\n"))
}

func (m *ChatModel) saveSession() {
	if m.session == nil || m.marlinDir == "" {
		return
	}
	m.session.Entries = make([]history.Entry, len(m.entries))
	for i, e := range m.entries {
		m.session.Entries[i] = history.Entry{
			Role:     e.role,
			Content:  e.content,
			Time:     e.time,
			ToolName: e.toolName,
			IsError:  e.isError,
		}
	}
	m.session.Messages = make([]history.Message, len(m.history))
	for i, msg := range m.history {
		var tcs []history.ToolCall
		for _, tc := range msg.ToolCalls {
			tcs = append(tcs, history.ToolCall{ID: tc.ID, Name: tc.Name, Input: tc.Input})
		}
		m.session.Messages[i] = history.Message{
			Role:       msg.Role,
			Content:    msg.Content,
			ToolCalls:  tcs,
			ToolCallID: msg.ToolCallID,
			ToolUseID:  msg.ToolUseID,
			IsError:    msg.IsError,
		}
	}
	history.SaveSession(m.marlinDir, m.session)
}

func sandboxBackendName(mgr *container.Manager) string {
	if mgr == nil {
		return "none"
	}
	return mgr.BackendName()
}

// tabComplete returns the longest common prefix of all suggestions that is
// longer than what the user has typed, enabling incremental Tab completion.
// If only one suggestion matches, it returns that command directly.
func tabComplete(typed string, suggs []cmdSuggestion) string {
	if len(suggs) == 0 {
		return ""
	}
	if len(suggs) == 1 {
		return suggs[0].cmd
	}
	prefix := suggs[0].cmd
	for _, s := range suggs[1:] {
		for !strings.HasPrefix(s.cmd, prefix) {
			if len(prefix) == 0 {
				return typed
			}
			prefix = prefix[:len(prefix)-1]
		}
	}
	if len(prefix) > len(typed) {
		return prefix
	}
	return typed
}

// rateBar renders a simple ASCII progress bar showing remaining wait time.
// pct=1.0 means full wait remaining, pct=0 means done.
func rateBar(pct float64, width int) string {
	filled := int(pct * float64(width))
	if filled > width {
		filled = width
	}
	empty := width - filled
	return "[" + strings.Repeat("█", filled) + strings.Repeat("░", empty) + "]"
}
