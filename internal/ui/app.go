package ui

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"marlin/internal/config"
	"marlin/internal/providers"
)

type viewState int

const (
	viewSplash viewState = iota
	viewChat
	viewEditor
)

type App struct {
	cfg       *config.Config
	registry  *providers.Registry
	state     viewState
	splash    SplashModel
	chat      ChatModel
	editor    EditorModel
	statusBar StatusBar
	width     int
	height    int
}

func NewApp(cfg *config.Config) App {
	registry := providers.NewRegistry(cfg)

	// Bootstrap with a default terminal size; WindowSizeMsg fixes it immediately.
	width, height := 120, 40

	return App{
		cfg:      cfg,
		registry: registry,
		state:    viewSplash,
		splash:   NewSplash(width, height),
		statusBar: NewStatusBar(width),
	}
}

func (a App) Init() tea.Cmd {
	return tea.Batch(
		tea.SetWindowTitle("Marlin"),
		a.splash.Init(),
	)
}

func (a App) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {

	case tea.WindowSizeMsg:
		a.width = msg.Width
		a.height = msg.Height
		a.statusBar.SetWidth(a.width)
		switch a.state {
		case viewSplash:
			a.splash = NewSplash(a.width, a.height)
		case viewChat:
			a.chat.Resize(a.width, a.height-1)
		case viewEditor:
			a.editor.Resize(a.width, a.height-1)
		}
		return a, nil

	case tea.KeyMsg:
		if msg.String() == "ctrl+q" && a.state == viewChat {
			return a, tea.Quit
		}

	case SplashDoneMsg:
		a.state = viewChat
		a.chat = NewChat(a.cfg, a.registry, a.width, a.height-1)
		a.statusBar.SetProvider(a.cfg.ActiveProvider, a.cfg.ActiveModel)
		a.statusBar.SetMode("chat")
		return a, a.chat.Init()

	case OpenEditorMsg:
		a.state = viewEditor
		a.editor = NewEditor(msg.Filename, msg.Readonly, a.width, a.height-1)
		a.statusBar.SetMode("editor")
		a.statusBar.SetInfo(msg.Filename)
		return a, a.editor.Init()

	case EditorCloseMsg:
		a.state = viewChat
		a.statusBar.SetMode("chat")
		a.statusBar.SetInfo("")
		a.statusBar.SetCursor(0, 0)
		return a, nil

	case SetStatusMsg:
		a.cfg.ActiveProvider = msg.Provider
		a.cfg.ActiveModel = msg.Model
		a.statusBar.SetProvider(msg.Provider, msg.Model)
		if msg.Info != "" {
			a.statusBar.SetInfo(msg.Info)
		}
		a.statusBar.SetSandbox(msg.SandboxActive, msg.SandboxBackend)
		return a, nil
	}

	// Delegate to active view.
	switch a.state {
	case viewSplash:
		var cmd tea.Cmd
		a.splash, cmd = a.splash.Update(msg)
		return a, cmd

	case viewChat:
		var cmd tea.Cmd
		a.chat, cmd = a.chat.Update(msg)
		return a, cmd

	case viewEditor:
		var cmd tea.Cmd
		a.editor, cmd = a.editor.Update(msg)
		if a.editor.IsModified() {
			a.statusBar.SetInfo(a.editor.Filename() + " ●")
		} else {
			a.statusBar.SetInfo(a.editor.Filename())
		}
		a.statusBar.SetCursor(a.editor.cursor.row+1, a.editor.cursor.col+1)
		return a, cmd
	}

	return a, nil
}

func (a App) View() string {
	var content string

	switch a.state {
	case viewSplash:
		return a.splash.View()
	case viewChat:
		content = a.chat.View()
	case viewEditor:
		content = a.editor.View()
	}

	statusView := a.statusBar.View()

	return lipgloss.JoinVertical(lipgloss.Left,
		lipgloss.NewStyle().
			Width(a.width).
			Height(a.height-1).
			Background(colDeepOcean).
			Render(content),
		statusView,
	)
}
