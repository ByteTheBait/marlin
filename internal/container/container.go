package container

import "os/exec"

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
func (m *Manager) Exec(cmd, workDir string) (string, error) {
	return m.backend.Exec(cmd, workDir)
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
