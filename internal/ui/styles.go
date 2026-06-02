package ui

import "github.com/charmbracelet/lipgloss"

// Blue palette
var (
	colDeepOcean = lipgloss.Color("#071E33")
	colNavy      = lipgloss.Color("#0A2B4A")
	colCobalt    = lipgloss.Color("#1A5276")
	colSky       = lipgloss.Color("#2E86C1")
	colAqua      = lipgloss.Color("#3498DB")
	colIce       = lipgloss.Color("#85C1E9")
	colFoam      = lipgloss.Color("#D6EAF8")
	colWhite     = lipgloss.Color("#EAF4FB")
	colSlate     = lipgloss.Color("#5D6D7E")
	colGreen     = lipgloss.Color("#27AE60")
	colRed       = lipgloss.Color("#E74C3C")
	colGold      = lipgloss.Color("#F1C40F")
	colPurple    = lipgloss.Color("#8E44AD")
)

var (
	// Status bar
	StyleStatusBar = lipgloss.NewStyle().
			Background(colNavy).
			Foreground(colIce)

	StyleStatusProvider = lipgloss.NewStyle().
				Background(colCobalt).
				Foreground(colWhite).
				Bold(true).
				Padding(0, 1)

	StyleStatusModel = lipgloss.NewStyle().
				Background(colSky).
				Foreground(colDeepOcean).
				Bold(true).
				Padding(0, 1)

	StyleStatusInfo = lipgloss.NewStyle().
			Background(colNavy).
			Foreground(colIce).
			Padding(0, 1)

	StyleStatusRight = lipgloss.NewStyle().
				Background(colNavy).
				Foreground(colSlate).
				Padding(0, 1)

	StyleSandboxChip = lipgloss.NewStyle().
				Background(colGreen).
				Foreground(colDeepOcean).
				Bold(true).
				Padding(0, 1)

	// Chat
	StyleUserLabel = lipgloss.NewStyle().
			Foreground(colAqua).
			Bold(true)

	StyleAssistantLabel = lipgloss.NewStyle().
				Foreground(colIce).
				Bold(true)

	StyleSystemMsg = lipgloss.NewStyle().
			Foreground(colSlate).
			Italic(true)

	StyleErrorMsg = lipgloss.NewStyle().
			Foreground(colRed).
			Bold(true)

	StyleSuccessMsg = lipgloss.NewStyle().
			Foreground(colGreen)

	StyleInputBorder = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(colCobalt)

	StyleInputFocused = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(colAqua)

	StyleCursor = lipgloss.NewStyle().
			Background(colAqua).
			Foreground(colDeepOcean)

	// Editor
	StyleLineNum = lipgloss.NewStyle().
			Foreground(colCobalt).
			Width(5).
			Align(lipgloss.Right).
			PaddingRight(1)

	StyleLineNumCurrent = lipgloss.NewStyle().
				Foreground(colAqua).
				Bold(true).
				Width(5).
				Align(lipgloss.Right).
				PaddingRight(1)

	StyleEditorCurrentLine = lipgloss.NewStyle().
				Background(colNavy)

	StyleEditorBorder = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(colSky)

	StyleEditorHint = lipgloss.NewStyle().
			Background(colNavy).
			Foreground(colSlate).
			Padding(0, 1)

	StyleEditorModified = lipgloss.NewStyle().
				Background(colNavy).
				Foreground(colGold).
				Bold(true).
				Padding(0, 1)

	StyleEditorFilename = lipgloss.NewStyle().
				Background(colNavy).
				Foreground(colWhite).
				Bold(true).
				Padding(0, 1)

	// Splash
	StyleSplashFish = lipgloss.NewStyle().
			Foreground(colAqua)

	StyleSplashWave = lipgloss.NewStyle().
			Foreground(colCobalt)

	StyleSplashTitle = lipgloss.NewStyle().
				Foreground(colIce).
				Bold(true)

	StyleSplashSub = lipgloss.NewStyle().
			Foreground(colSky)

	StyleSplashHint = lipgloss.NewStyle().
			Foreground(colSlate)

	// Help
	StyleHelpKey = lipgloss.NewStyle().
			Foreground(colAqua).
			Bold(true)

	StyleHelpDesc = lipgloss.NewStyle().
			Foreground(colSlate)
)
