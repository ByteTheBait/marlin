package container

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"
)

const dockerImage = "alpine:latest"

// DockerBackend runs commands inside an ephemeral Docker container.
// The project workDir is mounted at /workspace inside the container.
// Network access is enabled so package managers (apk, npm, pip) work.
// Only the mounted workDir is writable — the rest of the host filesystem is inaccessible.
type DockerBackend struct {
	containerID string
}

func (d *DockerBackend) Name() string    { return "docker" }
func (d *DockerBackend) IsRunning() bool { return d.containerID != "" }

func (d *DockerBackend) Start(workDir string) error {
	name := fmt.Sprintf("marlin-sandbox-%d", time.Now().Unix())

	// Pull quietly; ignore errors (image may already be present locally).
	exec.Command("docker", "pull", "-q", dockerImage).Run()

	out, err := exec.Command("docker", "run", "-d", "--rm",
		"--name", name,
		"-v", workDir+":/workspace",
		"--workdir", "/workspace",
		"--memory", "512m",
		"--cpus", "1",
		dockerImage,
		"tail", "-f", "/dev/null",
	).Output()
	if err != nil {
		return fmt.Errorf("docker run: %w — is Docker running?", err)
	}

	d.containerID = strings.TrimSpace(string(out))
	return nil
}

func (d *DockerBackend) Exec(cmd, workDir string) (string, error) {
	if d.containerID == "" {
		return "", fmt.Errorf("container not running")
	}
	// Expand ~ to the host home dir, then remap the host workDir to /workspace
	// so paths like ~/project/src/main.go or /Users/henry/project/src/main.go
	// resolve correctly inside the container.
	if home, err := os.UserHomeDir(); err == nil {
		cmd = strings.ReplaceAll(cmd, "~/", home+"/")
		cmd = strings.ReplaceAll(cmd, " ~", " "+home) // bare ~ not followed by /
	}
	if workDir != "" {
		cmd = strings.ReplaceAll(cmd, workDir, "/workspace")
	}
	out, err := exec.Command("docker", "exec", "-w", "/workspace",
		d.containerID, "sh", "-c", cmd).CombinedOutput()
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

func (d *DockerBackend) Stop() error {
	if d.containerID == "" {
		return nil
	}
	err := exec.Command("docker", "rm", "-f", d.containerID).Run()
	d.containerID = ""
	return err
}
