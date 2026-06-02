package history

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"
)

const MaxInputEntries = 500

// ---- Input history ----

type InputHistory struct {
	Entries []string `json:"entries"`
	path    string
}

func LoadInput(marlinDir string) *InputHistory {
	h := &InputHistory{path: filepath.Join(marlinDir, "input_history.json")}
	data, err := os.ReadFile(h.path)
	if err == nil {
		json.Unmarshal(data, h)
	}
	return h
}

func (h *InputHistory) Add(entry string) {
	if entry == "" {
		return
	}
	if len(h.Entries) > 0 && h.Entries[0] == entry {
		return // skip consecutive duplicates
	}
	h.Entries = append([]string{entry}, h.Entries...)
	if len(h.Entries) > MaxInputEntries {
		h.Entries = h.Entries[:MaxInputEntries]
	}
	data, _ := json.Marshal(h)
	os.WriteFile(h.path, data, 0600)
}

// ---- Chat sessions ----

type ToolCall struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	Input string `json:"input"`
}

type Message struct {
	Role       string     `json:"role"`
	Content    string     `json:"content"`
	ToolCalls  []ToolCall `json:"tool_calls,omitempty"`
	ToolCallID string     `json:"tool_call_id,omitempty"`
	ToolUseID  string     `json:"tool_use_id,omitempty"`
	IsError    bool       `json:"is_error,omitempty"`
}

type Entry struct {
	Role     string    `json:"role"`
	Content  string    `json:"content"`
	Time     time.Time `json:"time"`
	ToolName string    `json:"tool_name,omitempty"`
	IsError  bool      `json:"is_error,omitempty"`
}

type Session struct {
	ID        string    `json:"id"`
	Project   string    `json:"project"`
	WorkDir   string    `json:"work_dir"`
	Entries   []Entry   `json:"entries"`
	Messages  []Message `json:"messages"`
	CreatedAt time.Time `json:"created_at"`
	UpdatedAt time.Time `json:"updated_at"`
}

func NewSession(project, workDir string) *Session {
	now := time.Now()
	return &Session{
		ID:        now.Format("2006-01-02T15-04-05"),
		Project:   project,
		WorkDir:   workDir,
		CreatedAt: now,
		UpdatedAt: now,
	}
}

// Summary returns a short description for listing.
func (s *Session) Summary() string {
	msgCount := 0
	preview := ""
	for _, e := range s.Entries {
		if e.Role == "user" || e.Role == "assistant" {
			msgCount++
		}
		if e.Role == "user" && preview == "" {
			preview = e.Content
			if len(preview) > 60 {
				preview = preview[:57] + "..."
			}
		}
	}
	return fmt.Sprintf("%s  [%d msgs]  %s", s.UpdatedAt.Format("Jan 02 15:04"), msgCount, preview)
}

func sessionsDir(marlinDir string) string {
	dir := filepath.Join(marlinDir, "sessions")
	os.MkdirAll(dir, 0755)
	return dir
}

func SaveSession(marlinDir string, s *Session) {
	if s == nil {
		return
	}
	s.UpdatedAt = time.Now()
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return
	}
	os.WriteFile(filepath.Join(sessionsDir(marlinDir), s.ID+".json"), data, 0600)
}

func ListSessions(marlinDir string) ([]*Session, error) {
	dir := sessionsDir(marlinDir)
	fentries, err := os.ReadDir(dir)
	if err != nil {
		return nil, err
	}
	var sessions []*Session
	for _, fe := range fentries {
		if !strings.HasSuffix(fe.Name(), ".json") {
			continue
		}
		data, err := os.ReadFile(filepath.Join(dir, fe.Name()))
		if err != nil {
			continue
		}
		var s Session
		if err := json.Unmarshal(data, &s); err != nil {
			continue
		}
		sessions = append(sessions, &s)
	}
	sort.Slice(sessions, func(i, j int) bool {
		return sessions[i].UpdatedAt.After(sessions[j].UpdatedAt)
	})
	return sessions, nil
}

func ClearSessions(marlinDir string) error {
	dir := filepath.Join(marlinDir, "sessions")
	if err := os.RemoveAll(dir); err != nil {
		return err
	}
	return os.MkdirAll(dir, 0755)
}
