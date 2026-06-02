package container

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
)

// SandboxBackend uses macOS sandbox-exec to isolate commands.
// All host tools (go, node, python, etc.) are available since it runs on the
// host OS. Only file-write access is restricted: the AI can only write inside
// workDir and /tmp — the rest of the host filesystem is read-only or inaccessible.
type SandboxBackend struct {
	workDir     string
	profilePath string
	running     bool
}

func (s *SandboxBackend) Name() string    { return "sandbox-exec" }
func (s *SandboxBackend) IsRunning() bool { return s.running }

func (s *SandboxBackend) Start(workDir string) error {
	s.workDir = workDir

	// Allow reads everywhere (so compilers, tools work), but restrict writes.
	profile := fmt.Sprintf(`(version 1)
(allow default)
(deny file-write*)
(allow file-write*
    (subpath "%s")
    (subpath "/tmp")
    (subpath "/private/tmp")
    (subpath "/var/folders"))
`, workDir)

	f, err := os.CreateTemp("", "marlin-sandbox-*.sb")
	if err != nil {
		return fmt.Errorf("create sandbox profile: %w", err)
	}
	if _, err := f.WriteString(profile); err != nil {
		f.Close()
		os.Remove(f.Name())
		return err
	}
	f.Close()

	s.profilePath = f.Name()
	s.running = true
	return nil
}

func (s *SandboxBackend) Exec(cmd, workDir string) (string, error) {
	if !s.running {
		return "", fmt.Errorf("sandbox not started")
	}
	c := exec.Command("sandbox-exec", "-f", s.profilePath, "sh", "-c", cmd)
	c.Dir = workDir
	out, err := c.CombinedOutput()
	output := strings.TrimSpace(string(out))
	if output == "" && err != nil {
		output = err.Error()
	}
	if output == "" {
		output = "(no output)"
	}
	if err != nil {
		return output, err
	}
	return output, nil
}

func (s *SandboxBackend) Stop() error {
	s.running = false
	if s.profilePath != "" {
		os.Remove(s.profilePath)
		s.profilePath = ""
	}
	return nil
}
