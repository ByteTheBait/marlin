package ui

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"unicode/utf8"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

type EditorCloseMsg struct{}

type editorMode int

const (
	editorModeNormal editorMode = iota
	editorModeSearch
)

type EditorModel struct {
	filename string
	lines    []string
	cursor   struct{ row, col int }
	offset   int // first visible row
	undo     [][]string
	clipboard string
	modified bool
	readonly bool
	mode     editorMode
	search   string
	searchHL int // highlighted search result row (-1 if none)
	message  string
	width    int
	height   int
}

func NewEditor(filename string, readonly bool, width, height int) EditorModel {
	m := EditorModel{
		filename: filename,
		readonly: readonly,
		width:    width,
		height:   height,
		searchHL: -1,
	}
	m.load()
	return m
}

func (m *EditorModel) Resize(width, height int) {
	m.width = width
	m.height = height
	m.clampOffset()
}

func (m *EditorModel) load() {
	data, err := os.ReadFile(m.filename)
	if err != nil {
		m.lines = []string{""}
		return
	}
	text := string(data)
	text = strings.ReplaceAll(text, "\r\n", "\n")
	m.lines = strings.Split(text, "\n")
	if len(m.lines) == 0 {
		m.lines = []string{""}
	}
}

func (m *EditorModel) save() error {
	if m.readonly {
		return fmt.Errorf("file is read-only")
	}
	if err := os.MkdirAll(filepath.Dir(m.filename), 0755); err != nil {
		return err
	}
	content := strings.Join(m.lines, "\n")
	if err := os.WriteFile(m.filename, []byte(content), 0644); err != nil {
		return err
	}
	m.modified = false
	return nil
}

func (m EditorModel) Init() tea.Cmd { return nil }

func (m EditorModel) Update(msg tea.Msg) (EditorModel, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		if m.mode == editorModeSearch {
			return m.updateSearch(msg)
		}
		return m.updateNormal(msg)
	}
	return m, nil
}

func (m EditorModel) updateSearch(msg tea.KeyMsg) (EditorModel, tea.Cmd) {
	switch msg.Type {
	case tea.KeyEsc:
		m.mode = editorModeNormal
		m.search = ""
		m.searchHL = -1
		m.message = ""
	case tea.KeyEnter:
		m.mode = editorModeNormal
		m.message = ""
		if m.search != "" {
			m.findNext()
		}
	case tea.KeyBackspace:
		if len(m.search) > 0 {
			_, size := utf8.DecodeLastRuneInString(m.search)
			m.search = m.search[:len(m.search)-size]
		}
	default:
		if len(msg.Runes) > 0 {
			m.search += string(msg.Runes)
		}
	}
	return m, nil
}

func (m EditorModel) updateNormal(msg tea.KeyMsg) (EditorModel, tea.Cmd) {
	switch msg.String() {
	case "ctrl+q", "esc":
		return m, func() tea.Msg { return EditorCloseMsg{} }

	case "ctrl+s":
		if err := m.save(); err != nil {
			m.message = "Error: " + err.Error()
		} else {
			m.message = "Saved."
		}

	case "ctrl+f":
		m.mode = editorModeSearch
		m.search = ""
		m.message = ""

	case "ctrl+z":
		m.undo_()

	case "ctrl+c":
		if m.cursor.row < len(m.lines) {
			m.clipboard = m.lines[m.cursor.row]
		}
		m.message = "Line copied."

	case "ctrl+x":
		if m.readonly {
			m.message = "Read-only."
			break
		}
		if m.cursor.row < len(m.lines) {
			m.clipboard = m.lines[m.cursor.row]
			m.snapshot()
			m.lines = append(m.lines[:m.cursor.row], m.lines[m.cursor.row+1:]...)
			if len(m.lines) == 0 {
				m.lines = []string{""}
			}
			m.clampCursor()
			m.modified = true
			m.message = "Line cut."
		}

	case "ctrl+v":
		if m.readonly {
			m.message = "Read-only."
			break
		}
		if m.clipboard != "" {
			m.snapshot()
			newLines := make([]string, 0, len(m.lines)+1)
			newLines = append(newLines, m.lines[:m.cursor.row+1]...)
			newLines = append(newLines, m.clipboard)
			newLines = append(newLines, m.lines[m.cursor.row+1:]...)
			m.lines = newLines
			m.cursor.row++
			m.cursor.col = 0
			m.modified = true
		}

	case "ctrl+g":
		// Already handled via search — could add go-to-line here
		m.message = fmt.Sprintf("Line %d/%d", m.cursor.row+1, len(m.lines))

	case "up", "k":
		if m.cursor.row > 0 {
			m.cursor.row--
			m.clampCol()
			m.clampOffset()
		}

	case "down", "j":
		if m.cursor.row < len(m.lines)-1 {
			m.cursor.row++
			m.clampCol()
			m.clampOffset()
		}

	case "left", "h":
		if m.cursor.col > 0 {
			m.cursor.col--
		} else if m.cursor.row > 0 {
			m.cursor.row--
			m.cursor.col = len(m.lines[m.cursor.row])
			m.clampOffset()
		}

	case "right", "l":
		if m.cursor.row < len(m.lines) {
			line := m.lines[m.cursor.row]
			if m.cursor.col < len(line) {
				m.cursor.col++
			} else if m.cursor.row < len(m.lines)-1 {
				m.cursor.row++
				m.cursor.col = 0
				m.clampOffset()
			}
		}

	case "home", "ctrl+a":
		m.cursor.col = 0

	case "end", "ctrl+e":
		if m.cursor.row < len(m.lines) {
			m.cursor.col = len(m.lines[m.cursor.row])
		}

	case "pgup", "ctrl+u":
		m.cursor.row -= m.visibleLines()
		if m.cursor.row < 0 {
			m.cursor.row = 0
		}
		m.clampCol()
		m.clampOffset()

	case "pgdown", "ctrl+d":
		m.cursor.row += m.visibleLines()
		if m.cursor.row >= len(m.lines) {
			m.cursor.row = len(m.lines) - 1
		}
		m.clampCol()
		m.clampOffset()

	case "ctrl+home":
		m.cursor.row = 0
		m.cursor.col = 0
		m.offset = 0

	case "ctrl+end":
		m.cursor.row = len(m.lines) - 1
		m.cursor.col = len(m.lines[m.cursor.row])
		m.clampOffset()

	case "enter":
		if m.readonly {
			break
		}
		m.snapshot()
		line := m.lines[m.cursor.row]
		before := line[:m.cursor.col]
		after := line[m.cursor.col:]
		newLines := make([]string, 0, len(m.lines)+1)
		newLines = append(newLines, m.lines[:m.cursor.row]...)
		newLines = append(newLines, before, after)
		newLines = append(newLines, m.lines[m.cursor.row+1:]...)
		m.lines = newLines
		m.cursor.row++
		m.cursor.col = 0
		m.modified = true
		m.clampOffset()

	case "backspace":
		if m.readonly {
			break
		}
		m.snapshot()
		if m.cursor.col > 0 {
			line := m.lines[m.cursor.row]
			// find rune boundary
			runes := []rune(line)
			if m.cursor.col <= len(runes) {
				runes = append(runes[:m.cursor.col-1], runes[m.cursor.col:]...)
				m.lines[m.cursor.row] = string(runes)
				m.cursor.col--
			}
			m.modified = true
		} else if m.cursor.row > 0 {
			prev := m.lines[m.cursor.row-1]
			cur := m.lines[m.cursor.row]
			m.cursor.col = len(prev)
			m.lines[m.cursor.row-1] = prev + cur
			m.lines = append(m.lines[:m.cursor.row], m.lines[m.cursor.row+1:]...)
			m.cursor.row--
			m.modified = true
			m.clampOffset()
		}

	case "delete":
		if m.readonly {
			break
		}
		m.snapshot()
		line := m.lines[m.cursor.row]
		runes := []rune(line)
		if m.cursor.col < len(runes) {
			m.lines[m.cursor.row] = string(append(runes[:m.cursor.col], runes[m.cursor.col+1:]...))
			m.modified = true
		} else if m.cursor.row < len(m.lines)-1 {
			m.lines[m.cursor.row] = line + m.lines[m.cursor.row+1]
			m.lines = append(m.lines[:m.cursor.row+1], m.lines[m.cursor.row+2:]...)
			m.modified = true
		}

	case "tab":
		if m.readonly {
			break
		}
		m.snapshot()
		line := m.lines[m.cursor.row]
		runes := []rune(line)
		insert := []rune("    ") // 4 spaces
		newRunes := make([]rune, 0, len(runes)+4)
		newRunes = append(newRunes, runes[:m.cursor.col]...)
		newRunes = append(newRunes, insert...)
		newRunes = append(newRunes, runes[m.cursor.col:]...)
		m.lines[m.cursor.row] = string(newRunes)
		m.cursor.col += 4
		m.modified = true

	default:
		if m.readonly {
			break
		}
		if len(msg.Runes) > 0 {
			m.snapshot()
			line := m.lines[m.cursor.row]
			runes := []rune(line)
			newRunes := make([]rune, 0, len(runes)+len(msg.Runes))
			if m.cursor.col > len(runes) {
				m.cursor.col = len(runes)
			}
			newRunes = append(newRunes, runes[:m.cursor.col]...)
			newRunes = append(newRunes, msg.Runes...)
			newRunes = append(newRunes, runes[m.cursor.col:]...)
			m.lines[m.cursor.row] = string(newRunes)
			m.cursor.col += len(msg.Runes)
			m.modified = true
		}
	}

	return m, nil
}

func (m EditorModel) View() string {
	visible := m.visibleLines()
	lineNumW := len(fmt.Sprintf("%d", len(m.lines))) + 2

	var sb strings.Builder

	for i := 0; i < visible; i++ {
		row := m.offset + i
		if row >= len(m.lines) {
			// empty lines past end of file
			numStyle := StyleLineNum.Width(lineNumW)
			sb.WriteString(numStyle.Render("~") + "\n")
			continue
		}

		line := m.lines[row]
		lineNum := fmt.Sprintf("%d", row+1)
		isCurrent := row == m.cursor.row

		var numStyle lipgloss.Style
		if isCurrent {
			numStyle = StyleLineNumCurrent.Width(lineNumW)
		} else {
			numStyle = StyleLineNum.Width(lineNumW)
		}

		rendered := renderLine(line, m.cursor.col, isCurrent && m.mode == editorModeNormal)

		if isCurrent {
			lineContent := StyleEditorCurrentLine.Width(m.width - lineNumW - 2).Render(rendered)
			sb.WriteString(numStyle.Render(lineNum) + "│" + lineContent + "\n")
		} else {
			sb.WriteString(numStyle.Render(lineNum) + "│" + rendered + "\n")
		}
	}

	content := sb.String()

	// Bottom hint bar
	var hint string
	if m.mode == editorModeSearch {
		hint = StyleEditorHint.Render("Search: ") + m.search + "  [Enter:find  Esc:cancel]"
	} else if m.message != "" {
		hint = StyleEditorHint.Render(m.message)
	} else {
		modeLabel := "VIEW"
		if !m.readonly {
			modeLabel = "EDIT"
		}
		modStr := ""
		if m.modified {
			modStr = StyleEditorModified.Render(" ● ")
		}
		hint = StyleEditorFilename.Render(" "+filepath.Base(m.filename)+" ") +
			modStr +
			StyleEditorHint.Render(fmt.Sprintf(" %s  %d:%d  ", modeLabel, m.cursor.row+1, m.cursor.col+1)) +
			StyleEditorHint.Render(keybindHint(m.readonly))
	}

	return content + hint
}

func renderLine(line string, cursorCol int, showCursor bool) string {
	if !showCursor {
		return line
	}
	runes := []rune(line)
	if cursorCol >= len(runes) {
		return line + StyleCursor.Render(" ")
	}
	before := string(runes[:cursorCol])
	cur := StyleCursor.Render(string(runes[cursorCol]))
	after := string(runes[cursorCol+1:])
	return before + cur + after
}

func keybindHint(readonly bool) string {
	if readonly {
		return " ^Q:close  ^F:find  ↑↓←→:navigate  PgUp/PgDn:scroll "
	}
	return " ^S:save  ^Q:close  ^F:find  ^Z:undo  ^C:copy  ^X:cut  ^V:paste "
}

func (m *EditorModel) snapshot() {
	if len(m.undo) >= 50 {
		m.undo = m.undo[1:]
	}
	cp := make([]string, len(m.lines))
	copy(cp, m.lines)
	m.undo = append(m.undo, cp)
}

func (m *EditorModel) undo_() {
	if len(m.undo) == 0 {
		m.message = "Nothing to undo."
		return
	}
	last := m.undo[len(m.undo)-1]
	m.undo = m.undo[:len(m.undo)-1]
	m.lines = last
	m.clampCursor()
	m.modified = true
	m.message = "Undone."
}

func (m *EditorModel) findNext() {
	query := strings.ToLower(m.search)
	start := m.cursor.row + 1
	for i := 0; i < len(m.lines); i++ {
		row := (start + i) % len(m.lines)
		if strings.Contains(strings.ToLower(m.lines[row]), query) {
			m.cursor.row = row
			idx := strings.Index(strings.ToLower(m.lines[row]), query)
			m.cursor.col = idx
			m.searchHL = row
			m.clampOffset()
			m.message = fmt.Sprintf("Found at line %d.", row+1)
			return
		}
	}
	m.message = fmt.Sprintf("%q not found.", m.search)
}

func (m *EditorModel) visibleLines() int {
	h := m.height - 2 // minus hint bar and border
	if h < 1 {
		h = 1
	}
	return h
}

func (m *EditorModel) clampCursor() {
	if m.cursor.row >= len(m.lines) {
		m.cursor.row = len(m.lines) - 1
	}
	if m.cursor.row < 0 {
		m.cursor.row = 0
	}
	m.clampCol()
}

func (m *EditorModel) clampCol() {
	if m.cursor.row < len(m.lines) {
		maxCol := len([]rune(m.lines[m.cursor.row]))
		if m.cursor.col > maxCol {
			m.cursor.col = maxCol
		}
	}
}

func (m *EditorModel) clampOffset() {
	visible := m.visibleLines()
	if m.cursor.row < m.offset {
		m.offset = m.cursor.row
	}
	if m.cursor.row >= m.offset+visible {
		m.offset = m.cursor.row - visible + 1
	}
	if m.offset < 0 {
		m.offset = 0
	}
}

// IsModified reports whether there are unsaved changes.
func (m *EditorModel) IsModified() bool { return m.modified }

// Filename returns the editor's current filename.
func (m *EditorModel) Filename() string { return m.filename }
