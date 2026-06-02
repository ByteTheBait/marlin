package ui

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/charmbracelet/bubbles/viewport"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/glamour"
	"github.com/charmbracelet/lipgloss"
)

// OpenFileTreeMsg switches the app into the file tree view rooted at Dir.
type OpenFileTreeMsg struct{ Dir string }

// FileTreeCloseMsg returns to chat.
type FileTreeCloseMsg struct{}

type treeMode int

const (
	treeBrowse   treeMode = iota
	treeViewFile          // currently displaying a file's contents
)

// FileTreeModel is a full-screen file browser + viewer.
type FileTreeModel struct {
	root    string // starting directory (informational)
	current string // directory currently listed
	entries []os.DirEntry
	cursor  int // selected row index
	scrollY int // index of first visible row

	mode     treeMode
	filePath string
	fileVP   viewport.Model
	renderer *glamour.TermRenderer

	width  int
	height int
	errMsg string
}

func NewFileTree(dir string, width, height int) FileTreeModel {
	renderer, _ := glamour.NewTermRenderer(
		glamour.WithStylePath("dark"),
		glamour.WithWordWrap(width-4),
	)
	m := FileTreeModel{
		root:     dir,
		width:    width,
		height:   height,
		renderer: renderer,
	}
	m.fileVP = viewport.New(width-2, m.vpHeight())
	m.navigate(dir)
	return m
}

func (m FileTreeModel) Init() tea.Cmd { return nil }

func (m *FileTreeModel) Resize(width, height int) {
	m.width = width
	m.height = height
	m.fileVP.Width = width - 2
	m.fileVP.Height = m.vpHeight()
	m.renderer, _ = glamour.NewTermRenderer(
		glamour.WithStylePath("dark"),
		glamour.WithWordWrap(width-4),
	)
}

// listHeight is the number of rows available for the directory listing.
// Layout: header(1) + blank(1) + list(n) + hint(1) = height
func (m FileTreeModel) listHeight() int {
	h := m.height - 3
	if h < 1 {
		h = 1
	}
	return h
}

// vpHeight is the viewport height when viewing a file.
// Layout: header(1) + blank(1) + viewport(n) + blank(1) + hint(1) = height
func (m FileTreeModel) vpHeight() int {
	h := m.height - 4
	if h < 1 {
		h = 1
	}
	return h
}

func (m *FileTreeModel) navigate(dir string) {
	entries, err := os.ReadDir(dir)
	if err != nil {
		m.errMsg = err.Error()
		return
	}
	// Directories first, then files; both groups case-insensitive alphabetical.
	sort.Slice(entries, func(i, j int) bool {
		di, dj := entries[i].IsDir(), entries[j].IsDir()
		if di != dj {
			return di
		}
		return strings.ToLower(entries[i].Name()) < strings.ToLower(entries[j].Name())
	})
	m.current = dir
	m.entries = entries
	m.cursor = 0
	m.scrollY = 0
	m.errMsg = ""
}

func (m *FileTreeModel) adjustScroll() {
	visible := m.listHeight()
	if m.cursor < m.scrollY {
		m.scrollY = m.cursor
	} else if m.cursor >= m.scrollY+visible {
		m.scrollY = m.cursor - visible + 1
	}
	if m.scrollY < 0 {
		m.scrollY = 0
	}
}

func (m *FileTreeModel) openFile(path string) {
	data, err := os.ReadFile(path)
	if err != nil {
		m.errMsg = err.Error()
		return
	}
	content := string(data)
	ext := strings.ToLower(strings.TrimPrefix(filepath.Ext(path), "."))
	// Render markdown with glamour; show everything else as raw text.
	if (ext == "md" || ext == "markdown") && m.renderer != nil && len(data) < 200*1024 {
		if rendered, err := m.renderer.Render(content); err == nil {
			content = rendered
		}
	}
	m.filePath = path
	m.fileVP.SetContent(content)
	m.fileVP.GotoTop()
	m.mode = treeViewFile
	m.errMsg = ""
}

func (m FileTreeModel) Update(msg tea.Msg) (FileTreeModel, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		if m.mode == treeViewFile {
			switch msg.String() {
			case "q", "esc", "backspace", "left", "h":
				m.mode = treeBrowse
				return m, nil
			}
			var cmd tea.Cmd
			m.fileVP, cmd = m.fileVP.Update(msg)
			return m, cmd
		}

		// Browse mode.
		switch msg.String() {
		case "q", "esc":
			return m, func() tea.Msg { return FileTreeCloseMsg{} }

		case "up", "k":
			if m.cursor > 0 {
				m.cursor--
				m.adjustScroll()
			}

		case "down", "j":
			if m.cursor < len(m.entries)-1 {
				m.cursor++
				m.adjustScroll()
			}

		case "pgup":
			m.cursor -= m.listHeight()
			if m.cursor < 0 {
				m.cursor = 0
			}
			m.adjustScroll()

		case "pgdown":
			m.cursor += m.listHeight()
			if m.cursor >= len(m.entries) {
				m.cursor = len(m.entries) - 1
			}
			m.adjustScroll()

		case "g":
			m.cursor = 0
			m.scrollY = 0

		case "G":
			if len(m.entries) > 0 {
				m.cursor = len(m.entries) - 1
				m.adjustScroll()
			}

		case "enter", "right", "l":
			if len(m.entries) == 0 {
				break
			}
			e := m.entries[m.cursor]
			path := filepath.Join(m.current, e.Name())
			if e.IsDir() {
				m.navigate(path)
			} else {
				m.openFile(path)
			}

		case "backspace", "left", "h":
			parent := filepath.Dir(m.current)
			if parent == m.current {
				break // already at filesystem root
			}
			prevDir := filepath.Base(m.current)
			m.navigate(parent)
			// Re-position cursor on the directory we came from.
			for i, e := range m.entries {
				if e.Name() == prevDir {
					m.cursor = i
					m.adjustScroll()
					break
				}
			}
		}
	}
	return m, nil
}

func (m FileTreeModel) View() string {
	if m.mode == treeViewFile {
		return m.viewFile()
	}
	return m.viewTree()
}

func (m FileTreeModel) shortPath(p string) string {
	if home, err := os.UserHomeDir(); err == nil {
		p = strings.Replace(p, home, "~", 1)
	}
	return p
}

func (m FileTreeModel) viewTree() string {
	var sb strings.Builder

	// Header: current path.
	sb.WriteString(lipgloss.NewStyle().Foreground(colIce).Bold(true).Render("  "+m.shortPath(m.current)) + "\n")
	sb.WriteString("\n")

	if m.errMsg != "" {
		sb.WriteString(StyleErrorMsg.Render("  ✗ "+m.errMsg) + "\n")
		return sb.String()
	}

	visible := m.listHeight()

	if len(m.entries) == 0 {
		sb.WriteString(StyleSystemMsg.Render("  (empty directory)") + "\n")
		for i := 1; i < visible; i++ {
			sb.WriteString("\n")
		}
	} else {
		end := m.scrollY + visible
		if end > len(m.entries) {
			end = len(m.entries)
		}
		for i := m.scrollY; i < end; i++ {
			sb.WriteString(m.renderItem(m.entries[i], i == m.cursor) + "\n")
		}
		// Filler so the hint is always at the bottom.
		for i := end - m.scrollY; i < visible; i++ {
			sb.WriteString("\n")
		}
	}

	// Hint with scroll position.
	pos := fmt.Sprintf("%d/%d", m.cursor+1, len(m.entries))
	if m.scrollY > 0 {
		pos = "↑ " + pos
	}
	if below := len(m.entries) - (m.scrollY + visible); below > 0 {
		pos += " ↓"
	}
	sb.WriteString(StyleSystemMsg.Render("  ↑↓ navigate   Enter/→ open   ← back   q close   " + pos))

	return sb.String()
}

func (m FileTreeModel) viewFile() string {
	var sb strings.Builder
	sb.WriteString(lipgloss.NewStyle().Foreground(colIce).Bold(true).Render("  "+m.shortPath(m.filePath)) + "\n")
	sb.WriteString("\n")
	sb.WriteString(m.fileVP.View() + "\n")
	sb.WriteString("\n")
	sb.WriteString(StyleSystemMsg.Render("  ↑↓ scroll   PgUp/PgDn   ← back to tree   q close"))
	return sb.String()
}

func (m FileTreeModel) renderItem(e os.DirEntry, selected bool) string {
	name := e.Name()
	isHidden := strings.HasPrefix(name, ".")

	var prefix, display string
	if e.IsDir() {
		prefix = "▸ "
		display = name + "/"
	} else {
		prefix = "  "
		display = name
	}
	line := "  " + prefix + display

	var style lipgloss.Style
	switch {
	case selected:
		style = lipgloss.NewStyle().
			Background(colCobalt).
			Foreground(colWhite).
			Bold(true).
			Width(m.width)
	case isHidden:
		style = lipgloss.NewStyle().Foreground(colSlate).Width(m.width)
	case e.IsDir():
		style = lipgloss.NewStyle().Foreground(colAqua).Bold(true).Width(m.width)
	default:
		style = lipgloss.NewStyle().Foreground(colFoam).Width(m.width)
	}

	return style.Render(line)
}
