package info

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

type Info struct {
	Name        string            `json:"name,omitempty"`
	Description string            `json:"description,omitempty"`
	CWD         string            `json:"cwd,omitempty"`
	Stack       []string          `json:"stack,omitempty"`
	RunCmd      string            `json:"run,omitempty"`
	BuildCmd    string            `json:"build,omitempty"`
	TestCmd     string            `json:"test,omitempty"`
	Notes       string            `json:"notes,omitempty"`
	Commands    map[string]string `json:"commands,omitempty"`
	path        string
	dir         string
}

// Find walks up from startDir looking for info.json.
func Find(startDir string) (*Info, error) {
	dir := startDir
	for {
		p := filepath.Join(dir, "info.json")
		data, err := os.ReadFile(p)
		if err == nil {
			var info Info
			if err := json.Unmarshal(data, &info); err != nil {
				return nil, fmt.Errorf("info.json parse error: %w", err)
			}
			info.path = p
			info.dir = dir
			return &info, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return nil, fmt.Errorf("not found")
}

// New creates a bare Info anchored to dir (not yet saved).
func New(dir, name, description, runCmd string) *Info {
	return &Info{
		Name:        name,
		Description: description,
		CWD:         dir,
		RunCmd:      runCmd,
		path:        filepath.Join(dir, "info.json"),
		dir:         dir,
	}
}

// Save writes info.json, trimming Notes if the file would exceed 200 lines.
func (i *Info) Save() error {
	if i.path == "" {
		return fmt.Errorf("info has no path")
	}
	data, err := i.marshal()
	if err != nil {
		return err
	}
	if strings.Count(string(data), "\n")+1 > 200 && i.Notes != "" {
		// Trim notes until it fits
		for len(i.Notes) > 0 && strings.Count(string(data), "\n")+1 > 200 {
			cut := len(i.Notes) / 2
			if cut == 0 {
				i.Notes = ""
			} else {
				i.Notes = i.Notes[:cut] + "…"
			}
			data, err = i.marshal()
			if err != nil {
				return err
			}
		}
	}
	return os.WriteFile(i.path, data, 0644)
}

func (i *Info) marshal() ([]byte, error) {
	return json.MarshalIndent(i, "", "  ")
}

// MergeJSON applies a partial JSON object on top of the current Info fields.
// Only non-empty string fields and non-nil slice/map fields overwrite existing values.
func (i *Info) MergeJSON(raw string) error {
	var patch map[string]json.RawMessage
	if err := json.Unmarshal([]byte(raw), &patch); err != nil {
		return err
	}
	// Re-marshal the current Info, patch it, re-unmarshal.
	cur, err := i.marshal()
	if err != nil {
		return err
	}
	var merged map[string]json.RawMessage
	if err := json.Unmarshal(cur, &merged); err != nil {
		return err
	}
	for k, v := range patch {
		merged[k] = v
	}
	mergedBytes, err := json.Marshal(merged)
	if err != nil {
		return err
	}
	// Preserve path/dir across the unmarshal
	path, dir := i.path, i.dir
	if err := json.Unmarshal(mergedBytes, i); err != nil {
		return err
	}
	i.path, i.dir = path, dir
	return nil
}

// SetCWD updates the cwd field and saves.
func (i *Info) SetCWD(cwd string) error {
	i.CWD = cwd
	return i.Save()
}

// ForSystemPrompt returns a compact JSON string suitable for injection into a system prompt.
func (i *Info) ForSystemPrompt() string {
	data, err := i.marshal()
	if err != nil {
		return ""
	}
	return string(data)
}

// Execute runs the named command (run/build/test or a custom key).
func (i *Info) Execute(cmdName string) (string, error) {
	cmdStr := i.resolve(cmdName)
	if cmdStr == "" {
		return "", fmt.Errorf("command %q not defined in info.json", cmdName)
	}
	cmd := exec.Command("sh", "-c", cmdStr)
	cmd.Dir = i.dir
	out, err := cmd.CombinedOutput()
	return strings.TrimSpace(string(out)), err
}

// Dir returns the directory containing info.json.
func (i *Info) Dir() string  { return i.dir }
func (i *Info) Path() string { return i.path }

// AvailableCommands returns all defined command names.
func (i *Info) AvailableCommands() []string {
	var cmds []string
	if i.RunCmd != "" {
		cmds = append(cmds, "run")
	}
	if i.BuildCmd != "" {
		cmds = append(cmds, "build")
	}
	if i.TestCmd != "" {
		cmds = append(cmds, "test")
	}
	for k := range i.Commands {
		cmds = append(cmds, k)
	}
	return cmds
}

func (i *Info) resolve(name string) string {
	switch name {
	case "run":
		return i.RunCmd
	case "build":
		return i.BuildCmd
	case "test":
		return i.TestCmd
	default:
		if i.Commands != nil {
			return i.Commands[name]
		}
		return ""
	}
}

// Create writes a new info.json in dir (legacy helper).
func Create(dir, name, runCmd string) error {
	i := New(dir, name, "", runCmd)
	return i.Save()
}
