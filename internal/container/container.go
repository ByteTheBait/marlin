package container

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

// Backend executes commands in an isolated environment.
type Backend interface {
	Start(workDir string) error
	Exec(cmd, workDir string) (string, error)
	Stop() error
	IsRunning() bool
	Name() string
}

// Manager wraps a Backend with lifecycle management.
type Manager struct {
	backend Backend
}

// New returns a Manager with the best available backend, or nil if none available.
func New() *Manager {
	if dockerAvailable() {
		return &Manager{backend: &DockerBackend{}}
	}
	if sandboxExecAvailable() {
		return &Manager{backend: &SandboxBackend{}}
	}
	return nil
}

// Start initializes the sandbox bound to workDir.
func (m *Manager) Start(workDir string) error {
	return m.backend.Start(workDir)
}

// Exec runs cmd inside the sandbox with workDir as the working directory.
// After the command completes it diffs the directory state against the pre-run
// snapshot and appends a notice if new files were generated (e.g. dist/, swagger.json).
func (m *Manager) Exec(cmd, workDir string) (string, error) {
	before := snapshotDir(workDir)
	output, err := m.backend.Exec(cmd, workDir)
	after := snapshotDir(workDir)

	var newFiles []string
	for path := range after {
		if !before[path] {
			newFiles = append(newFiles, path)
		}
	}
	if len(newFiles) > 0 {
		const max = 8
		shown := newFiles
		suffix := ""
		if len(shown) > max {
			shown = shown[:max]
			suffix = fmt.Sprintf(" (and %d more)", len(newFiles)-max)
		}
		notice := "\n[sandbox] new files: " + strings.Join(shown, ", ") + suffix
		output += notice
	}
	return output, err
}

// snapshotDir returns a set of relative file paths in dir (non-recursive depth-2).
func snapshotDir(dir string) map[string]bool {
	result := map[string]bool{}
	entries, err := os.ReadDir(dir)
	if err != nil {
		return result
	}
	for _, e := range entries {
		rel := e.Name()
		result[rel] = true
		if e.IsDir() {
			sub := filepath.Join(dir, e.Name())
			if subEntries, err := os.ReadDir(sub); err == nil {
				for _, se := range subEntries {
					result[rel+"/"+se.Name()] = true
				}
			}
		}
	}
	return result
}

// Stop tears down the sandbox and releases resources.
func (m *Manager) Stop() error {
	return m.backend.Stop()
}

// IsRunning reports whether the sandbox is active.
func (m *Manager) IsRunning() bool {
	return m.backend != nil && m.backend.IsRunning()
}

// BackendName returns the name of the active backend.
func (m *Manager) BackendName() string {
	if m.backend == nil {
		return "none"
	}
	return m.backend.Name()
}

// Detect returns the name of the best available backend without starting it.
func Detect() string {
	if dockerAvailable() {
		return "docker"
	}
	if sandboxExecAvailable() {
		return "sandbox-exec"
	}
	return "none"
}

func dockerAvailable() bool {
	_, err := exec.LookPath("docker")
	return err == nil
}

func sandboxExecAvailable() bool {
	_, err := exec.LookPath("sandbox-exec")
	return err == nil
}
