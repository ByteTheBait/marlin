package ui

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"marlin/internal/providers"
	"marlin/internal/tools"
)

// ---- Message types ----

// OpenDiffMsg is sent by ChatModel to trigger the diff-review view.
type OpenDiffMsg struct{ Diffs []PendingDiff }

// DiffReviewDoneMsg is sent by DiffModel when the user finishes reviewing.
// Accepted/Rejected hold indices into the original Diffs slice.
type DiffReviewDoneMsg struct {
	Accepted []int
	Rejected []int
}

// ---- Diff data types ----

type diffLineType int

const (
	diffEqual  diffLineType = iota
	diffDelete              // line removed  (red -)
	diffInsert              // line added    (green +)
)

type diffLine struct {
	Type diffLineType
	Text string
}

// PendingDiff holds a pre-computed diff for one write_file or edit_file call.
type PendingDiff struct {
	Call       providers.ToolCall
	FilePath   string
	OldContent string
	NewContent string
	Lines      []diffLine
	Stats      string // e.g. "+12 -4"
}

// ComputeDiffForCall reads the current file and computes the pending diff
// for a write_file or edit_file tool call. Returns nil if the call is not
// a file-write or if the input cannot be parsed.
func ComputeDiffForCall(call providers.ToolCall, workDir string) *PendingDiff {
	if call.Name != "write_file" && call.Name != "edit_file" {
		return nil
	}
	var input map[string]string
	if err := json.Unmarshal([]byte(call.Input), &input); err != nil {
		return nil
	}

	resolve := func(p string) string {
		if strings.HasPrefix(p, "~/") {
			if home, err := os.UserHomeDir(); err == nil {
				return filepath.Join(home, p[2:])
			}
		}
		if filepath.IsAbs(p) {
			return p
		}
		return filepath.Join(workDir, p)
	}

	var filePath, oldContent, newContent string
	switch call.Name {
	case "write_file":
		filePath = resolve(input["path"])
		newContent = input["content"]
		if data, err := os.ReadFile(filePath); err == nil {
			oldContent = string(data)
		}
	case "edit_file":
		filePath = resolve(input["path"])
		data, err := os.ReadFile(filePath)
		if err != nil {
			return nil
		}
		oldContent = string(data)
		newContent = strings.Replace(oldContent, input["old_string"], input["new_string"], 1)
	}

	lines := computeLineDiff(oldContent, newContent)
	return &PendingDiff{
		Call:       call,
		FilePath:   filePath,
		OldContent: oldContent,
		NewContent: newContent,
		Lines:      lines,
		Stats:      diffStats(lines),
	}
}

// computeLineDiff produces a line-level diff using LCS.
// Falls back to full-replace for very large files.
func computeLineDiff(oldContent, newContent string) []diffLine {
	old := strings.Split(oldContent, "\n")
	new := strings.Split(newContent, "\n")

	const maxLines = 600
	if len(old) > maxLines || len(new) > maxLines {
		var r []diffLine
		for _, l := range old {
			r = append(r, diffLine{diffDelete, l})
		}
		for _, l := range new {
			r = append(r, diffLine{diffInsert, l})
		}
		return r
	}
	return lcsLines(old, new)
}

func lcsLines(old, new []string) []diffLine {
	m, n := len(old), len(new)
	dp := make([][]int, m+1)
	for i := range dp {
		dp[i] = make([]int, n+1)
	}
	for i := m - 1; i >= 0; i-- {
		for j := n - 1; j >= 0; j-- {
			if old[i] == new[j] {
				dp[i][j] = 1 + dp[i+1][j+1]
			} else if dp[i+1][j] >= dp[i][j+1] {
				dp[i][j] = dp[i+1][j]
			} else {
				dp[i][j] = dp[i][j+1]
			}
		}
	}
	var result []diffLine
	i, j := 0, 0
	for i < m && j < n {
		if old[i] == new[j] {
			result = append(result, diffLine{diffEqual, old[i]})
			i++
			j++
		} else if dp[i+1][j] >= dp[i][j+1] {
			result = append(result, diffLine{diffDelete, old[i]})
			i++
		} else {
			result = append(result, diffLine{diffInsert, new[j]})
			j++
		}
	}
	for ; i < m; i++ {
		result = append(result, diffLine{diffDelete, old[i]})
	}
	for ; j < n; j++ {
		result = append(result, diffLine{diffInsert, new[j]})
	}
	return result
}

func diffStats(lines []diffLine) string {
	adds, dels := 0, 0
	for _, l := range lines {
		switch l.Type {
		case diffInsert:
			adds++
		case diffDelete:
			dels++
		}
	}
	if adds == 0 && dels == 0 {
		return "no changes"
	}
	var parts []string
	if adds > 0 {
		parts = append(parts, fmt.Sprintf("+%d", adds))
	}
	if dels > 0 {
		parts = append(parts, fmt.Sprintf("-%d", dels))
	}
	return strings.Join(parts, " ")
}

// renderDiffContent builds the colored diff string for the viewport.
// Shows 3 context lines around each change; skips untouched stretches.
func renderDiffContent(lines []diffLine, width int) string {
	if len(lines) == 0 {
		return StyleSystemMsg.Render("  (empty)")
	}

	const ctx = 3
	show := make([]bool, len(lines))
	for i, l := range lines {
		if l.Type != diffEqual {
			for d := -ctx; d <= ctx; d++ {
				if j := i + d; j >= 0 && j < len(lines) {
					show[j] = true
				}
			}
		}
	}

	// Check if there are any actual changes.
	hasChanges := false
	for _, s := range show {
		if s {
			hasChanges = true
			break
		}
	}
	if !hasChanges {
		return StyleSystemMsg.Render("  (no changes)")
	}

	var sb strings.Builder
	prevShown := -1
	for i, l := range lines {
		if !show[i] {
			continue
		}
		if prevShown >= 0 && i > prevShown+1 {
			skipped := i - prevShown - 1
			sb.WriteString(lipgloss.NewStyle().Foreground(colCobalt).
				Render(fmt.Sprintf("@@ … %d unchanged lines …", skipped)) + "\n")
		}
		switch l.Type {
		case diffDelete:
			sb.WriteString(lipgloss.NewStyle().Foreground(colRed).
				Render("- "+l.Text) + "\n")
		case diffInsert:
			sb.WriteString(lipgloss.NewStyle().Foreground(colGreen).
				Render("+ "+l.Text) + "\n")
		case diffEqual:
			sb.WriteString(StyleSystemMsg.Render("  "+l.Text) + "\n")
		}
		prevShown = i
	}
	return sb.String()
}

// ---- DiffModel ----

// DiffModel is a full-screen diff reviewer. It shows one PendingDiff at a
// time and collects the user's accept/reject decisions.
type DiffModel struct {
	diffs   []PendingDiff
	current int
	decided []int8 // 0=pending, 1=accepted, 2=rejected

	vp     viewport.Model
	width  int
	height int
}

func NewDiffModel(diffs []PendingDiff, width, height int) DiffModel {
	m := DiffModel{
		diffs:   diffs,
		decided: make([]int8, len(diffs)),
		width:   width,
		height:  height,
	}
	m.vp = viewport.New(width-2, m.vpHeight())
	if len(diffs) > 0 {
		m.vp.SetContent(renderDiffContent(diffs[0].Lines, width))
	}
	return m
}

func (m DiffModel) Init() tea.Cmd { return nil }

func (m *DiffModel) Resize(width, height int) {
	m.width = width
	m.height = height
	m.vp.Width = width - 2
	m.vp.Height = m.vpHeight()
}

func (m DiffModel) vpHeight() int {
	h := m.height - 5 // header(2) + blank(1) + hint(2)
	if h < 1 {
		h = 1
	}
	return h
}

func (m *DiffModel) loadCurrent() {
	if m.current < len(m.diffs) {
		m.vp.SetContent(renderDiffContent(m.diffs[m.current].Lines, m.width))
		m.vp.GotoTop()
	}
}

func (m DiffModel) done() tea.Cmd {
	var accepted, rejected []int
	for i, d := range m.decided {
		switch d {
		case 1:
			accepted = append(accepted, i)
		case 0, 2: // pending treated as rejected
			rejected = append(rejected, i)
		}
	}
	return func() tea.Msg { return DiffReviewDoneMsg{Accepted: accepted, Rejected: rejected} }
}

func (m DiffModel) Update(msg tea.Msg) (DiffModel, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "a", "y": // accept
			if m.current < len(m.diffs) {
				m.decided[m.current] = 1
				m.current++
				if m.current >= len(m.diffs) {
					return m, m.done()
				}
				m.loadCurrent()
			}

		case "r", "n": // reject
			if m.current < len(m.diffs) {
				m.decided[m.current] = 2
				m.current++
				if m.current >= len(m.diffs) {
					return m, m.done()
				}
				m.loadCurrent()
			}

		case "q", "esc": // reject all remaining and close
			for i := m.current; i < len(m.diffs); i++ {
				if m.decided[i] == 0 {
					m.decided[i] = 2
				}
			}
			return m, m.done()

		default:
			var cmd tea.Cmd
			m.vp, cmd = m.vp.Update(msg)
			return m, cmd
		}
	}
	return m, nil
}

func (m DiffModel) View() string {
	if m.current >= len(m.diffs) {
		return StyleSystemMsg.Render("  Processing…")
	}
	pd := m.diffs[m.current]

	// Header
	relPath := pd.FilePath
	if home, err := os.UserHomeDir(); err == nil {
		relPath = strings.Replace(relPath, home, "~", 1)
	}
	counter := fmt.Sprintf("(%d/%d)", m.current+1, len(m.diffs))
	header := lipgloss.NewStyle().Foreground(colIce).Bold(true).
		Render(fmt.Sprintf("  %s  %s", relPath, counter))
	stats := lipgloss.NewStyle().Foreground(colSlate).
		Render(fmt.Sprintf("  %s  [%s]", pd.Call.Name, pd.Stats))

	hint := StyleSystemMsg.Render("  a/y accept   r/n reject   q reject remaining   ↑↓ scroll")

	return lipgloss.JoinVertical(lipgloss.Left,
		header,
		stats,
		"",
		m.vp.View(),
		"",
		hint,
	)
}

// ---- pendingDiffContext (stored in ChatModel while diff view is open) ----

type pendingDiffContext struct {
	diffs     []PendingDiff
	immediate []toolExecResult
	allCalls  []providers.ToolCall
	workDir   string
	allowed   func(string) bool
	cexec     func(string, string) (string, error)
	snapFn    func(string, string)
}

// diffApprovalNeededMsg is returned by the tool-execution goroutine when
// diff mode is on and at least one write call needs approval.
type diffApprovalNeededMsg struct {
	diffs     []PendingDiff
	immediate []toolExecResult
	allCalls  []providers.ToolCall
	workDir   string
	allowed   func(string) bool
	cexec     func(string, string) (string, error)
	snapFn    func(string, string)
}

// executeDiffApproved runs the approved writes and returns all results.
func executeDiffApproved(ctx *pendingDiffContext, accepted, rejected []int) []toolExecResult {
	results := make([]toolExecResult, 0, len(ctx.immediate)+len(ctx.diffs))
	results = append(results, ctx.immediate...)

	// Build a map so we output in original call order.
	decisions := make(map[int]bool, len(accepted))
	for _, i := range accepted {
		decisions[i] = true
	}

	for i, pd := range ctx.diffs {
		if decisions[i] {
			res := tools.Execute(pd.Call.Name, pd.Call.Input, ctx.workDir, ctx.allowed, ctx.cexec, ctx.snapFn)
			results = append(results, toolExecResult{call: pd.Call, output: res.Output, isError: res.IsError})
		} else {
			relPath := pd.FilePath
			results = append(results, toolExecResult{
				call:    pd.Call,
				output:  fmt.Sprintf("Change to %s was rejected by the user. Revise your approach or ask what they want.", relPath),
				isError: true,
			})
		}
	}
	return results
}
