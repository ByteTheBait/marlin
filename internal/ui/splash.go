package ui

import (
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

const logoText = `
  ███╗   ███╗ █████╗ ██████╗ ██╗     ██╗███╗   ██╗
  ████╗ ████║██╔══██╗██╔══██╗██║     ██║████╗  ██║
  ██╔████╔██║███████║██████╔╝██║     ██║██╔██╗ ██║
  ██║╚██╔╝██║██╔══██║██╔══██╗██║     ██║██║╚██╗██║
  ██║ ╚═╝ ██║██║  ██║██║  ██║███████╗██║██║ ╚████║
  ╚═╝     ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚══════╝╚═╝╚═╝  ╚═══╝`

type splashTickMsg struct{}
type SplashDoneMsg struct{}

type SplashModel struct {
	width  int
	height int
	ticks  int
}

func NewSplash(width, height int) SplashModel {
	return SplashModel{width: width, height: height}
}

func (m SplashModel) Init() tea.Cmd {
	return tickSplash()
}

func tickSplash() tea.Cmd {
	return tea.Tick(80*time.Millisecond, func(_ time.Time) tea.Msg {
		return splashTickMsg{}
	})
}

func (m SplashModel) Update(msg tea.Msg) (SplashModel, tea.Cmd) {
	switch msg.(type) {
	case splashTickMsg:
		m.ticks++
		if m.ticks >= 30 { // ~2.4 seconds
			return m, func() tea.Msg { return SplashDoneMsg{} }
		}
		return m, tickSplash()
	case tea.KeyMsg:
		return m, func() tea.Msg { return SplashDoneMsg{} }
	}
	return m, nil
}

func (m SplashModel) View() string {
	logo := StyleSplashTitle.Render(logoText)
	sub := StyleSplashSub.Render("  ai coding assistant · blue waters, sharp fins")
	hint := StyleSplashHint.Render("  press any key or wait to dive in...")

	content := lipgloss.JoinVertical(lipgloss.Left,
		logo,
		sub,
		"",
		hint,
	)

	return lipgloss.Place(m.width, m.height, lipgloss.Center, lipgloss.Center, content,
		lipgloss.WithWhitespaceBackground(colDeepOcean),
	)
}
