package ui

import (
	"fmt"
	"time"

	"github.com/charmbracelet/lipgloss"
)

type StatusBar struct {
	width      int
	provider   string
	model      string
	info       string // center info: filename, message, etc.
	mode       string
	line       int
	col        int
	sandbox    bool
	sandboxBak string // backend name for display
}

func NewStatusBar(width int) StatusBar {
	return StatusBar{
		width:    width,
		provider: "claude",
		model:    "claude-sonnet-4-5",
		mode:     "chat",
	}
}

func (s *StatusBar) SetWidth(w int) { s.width = w }

func (s *StatusBar) SetProvider(p, m string) {
	s.provider = p
	s.model = m
}

func (s *StatusBar) SetInfo(info string) { s.info = info }
func (s *StatusBar) SetMode(mode string) { s.mode = mode }
func (s *StatusBar) SetSandbox(active bool, backend string) {
	s.sandbox = active
	s.sandboxBak = backend
}
func (s *StatusBar) SetCursor(line, col int) {
	s.line = line
	s.col = col
}

func (s StatusBar) View() string {
	now := time.Now().Format("15:04")

	left := StyleStatusProvider.Render(" " + s.provider + " ") +
		StyleStatusModel.Render(" " + s.model + " ")
	if s.sandbox {
		bak := s.sandboxBak
		if bak == "" {
			bak = "sandbox"
		}
		left += StyleSandboxChip.Render(" SANDBOX·" + bak + " ")
	}

	var center string
	if s.info != "" {
		center = StyleStatusInfo.Render(s.info)
	}
	if s.line > 0 {
		center += StyleStatusInfo.Render(fmt.Sprintf("  %d:%d", s.line, s.col))
	}

	right := StyleStatusRight.Render(s.mode + " · " + now + " ")

	leftW := lipgloss.Width(left)
	rightW := lipgloss.Width(right)
	centerW := s.width - leftW - rightW
	if centerW < 0 {
		centerW = 0
	}

	centerPad := lipgloss.NewStyle().
		Background(colNavy).
		Width(centerW).
		Render(center)

	return left + centerPad + right
}
