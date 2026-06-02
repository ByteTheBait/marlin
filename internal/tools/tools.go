package tools

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// Def is a provider-agnostic tool definition.
type Def struct {
	Name        string
	Description string
	Properties  map[string]Prop
	Required    []string
}

// Prop describes a single input parameter.
type Prop struct {
	Type        string
	Description string
}

// Result holds the output of a single tool execution.
type Result struct {
	Output  string
	IsError bool
}

const maxOutputBytes = 40_000

// All returns every tool available to the model.
func All() []Def {
	return []Def{
		{
			Name:        "read_file",
			Description: "Read the complete contents of a file.",
			Properties: map[string]Prop{
				"path": {Type: "string", Description: "File path, relative to working directory or absolute."},
			},
			Required: []string{"path"},
		},
		{
			Name:        "write_file",
			Description: "Write content to a file, creating it (and any missing parent directories) if needed. Overwrites existing content.",
			Properties: map[string]Prop{
				"path":    {Type: "string", Description: "File path."},
				"content": {Type: "string", Description: "Full content to write."},
			},
			Required: []string{"path", "content"},
		},
		{
			Name:        "edit_file",
			Description: "Replace the first occurrence of old_string with new_string in a file. Preferred over write_file for targeted edits.",
			Properties: map[string]Prop{
				"path":       {Type: "string", Description: "File path."},
				"old_string": {Type: "string", Description: "Exact string to find. Must match the file content exactly."},
				"new_string": {Type: "string", Description: "Replacement string."},
			},
			Required: []string{"path", "old_string", "new_string"},
		},
		{
			Name:        "run_command",
			Description: "Run a shell command in the working directory and return combined stdout/stderr.",
			Properties: map[string]Prop{
				"command": {Type: "string", Description: "Shell command to execute."},
			},
			Required: []string{"command"},
		},
		{
			Name:        "list_directory",
			Description: "List files and subdirectories at a path.",
			Properties: map[string]Prop{
				"path": {Type: "string", Description: "Directory path. Uses working directory if omitted."},
			},
			Required: []string{},
		},
		{
			Name:        "create_directory",
			Description: "Create a directory and any necessary parent directories.",
			Properties: map[string]Prop{
				"path": {Type: "string", Description: "Directory path to create."},
			},
			Required: []string{"path"},
		},
	}
}

// Execute runs the named tool with the given JSON-encoded input object.
// isAllowed gates run_command in normal mode.
// containerExec, when non-nil, routes run_command through a sandbox instead of
// the host shell — the allow-list is bypassed when a container is active.
func Execute(name, inputJSON, workDir string, isAllowed func(string) bool, containerExec func(cmd, workDir string) (string, error)) Result {
	var input map[string]string
	if err := json.Unmarshal([]byte(inputJSON), &input); err != nil {
		// Try with interface{} and coerce
		var raw map[string]interface{}
		if err2 := json.Unmarshal([]byte(inputJSON), &raw); err2 != nil {
			return Result{Output: "input parse error: " + err.Error(), IsError: true}
		}
		input = make(map[string]string, len(raw))
		for k, v := range raw {
			switch val := v.(type) {
			case string:
				input[k] = val
			default:
				b, _ := json.Marshal(v)
				input[k] = string(b)
			}
		}
	}

	resolve := func(p string) string {
		if strings.HasPrefix(p, "~/") || p == "~" {
			if home, err := os.UserHomeDir(); err == nil {
				if p == "~" {
					return home
				}
				return filepath.Join(home, p[2:])
			}
		}
		if p == "" || filepath.IsAbs(p) {
			return p
		}
		return filepath.Join(workDir, p)
	}

	clamp := func(s string) string {
		if len(s) > maxOutputBytes {
			return s[:maxOutputBytes] + "\n…(truncated)"
		}
		return s
	}

	switch name {
	case "read_file":
		path := resolve(input["path"])
		data, err := os.ReadFile(path)
		if err != nil {
			return Result{Output: err.Error(), IsError: true}
		}
		return Result{Output: clamp(string(data))}

	case "write_file":
		path := resolve(input["path"])
		if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
			return Result{Output: err.Error(), IsError: true}
		}
		content := input["content"]
		if err := os.WriteFile(path, []byte(content), 0644); err != nil {
			return Result{Output: err.Error(), IsError: true}
		}
		return Result{Output: fmt.Sprintf("wrote %d bytes → %s", len(content), path)}

	case "edit_file":
		path := resolve(input["path"])
		raw, err := os.ReadFile(path)
		if err != nil {
			return Result{Output: err.Error(), IsError: true}
		}
		original := string(raw)
		oldStr := input["old_string"]
		if !strings.Contains(original, oldStr) {
			return Result{Output: "old_string not found in file", IsError: true}
		}
		updated := strings.Replace(original, oldStr, input["new_string"], 1)
		if err := os.WriteFile(path, []byte(updated), 0644); err != nil {
			return Result{Output: err.Error(), IsError: true}
		}
		return Result{Output: "edited " + path}

	case "run_command":
		cmd := input["command"]
		if containerExec != nil {
			// Sandbox mode — bypass allow-list, execute inside the container.
			output, err := containerExec(cmd, workDir)
			output = clamp(output)
			if err != nil {
				return Result{Output: output, IsError: true}
			}
			return Result{Output: output}
		}
		if !isAllowed(cmd) {
			first := cmd
			if parts := strings.Fields(cmd); len(parts) > 0 {
				first = parts[0]
			}
			return Result{
				Output:  fmt.Sprintf("not permitted: %q — use /allow %s or /sandbox on for autonomous mode", cmd, first),
				IsError: true,
			}
		}
		c := exec.Command("sh", "-c", cmd)
		c.Dir = workDir
		out, err := c.CombinedOutput()
		output := clamp(strings.TrimSpace(string(out)))
		if err != nil {
			if output == "" {
				output = err.Error()
			}
			return Result{Output: output, IsError: true}
		}
		if output == "" {
			output = "(no output)"
		}
		return Result{Output: output}

	case "list_directory":
		dir := workDir
		if p := resolve(input["path"]); p != "" {
			dir = p
		}
		entries, err := os.ReadDir(dir)
		if err != nil {
			return Result{Output: err.Error(), IsError: true}
		}
		var lines []string
		for _, e := range entries {
			if e.IsDir() {
				lines = append(lines, e.Name()+"/")
			} else {
				lines = append(lines, e.Name())
			}
		}
		if len(lines) == 0 {
			return Result{Output: "(empty directory)"}
		}
		return Result{Output: strings.Join(lines, "\n")}

	case "create_directory":
		path := resolve(input["path"])
		if err := os.MkdirAll(path, 0755); err != nil {
			return Result{Output: err.Error(), IsError: true}
		}
		return Result{Output: "created " + path}

	default:
		return Result{Output: "unknown tool: " + name, IsError: true}
	}
}
