package snapshots

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"time"
)

// Snapshot records one point-in-time copy of a file before it was modified.
type Snapshot struct {
	ID        string    `json:"id"`
	File      string    `json:"file"`     // path relative to workDir
	AbsFile   string    `json:"abs_file"` // absolute path at time of snapshot
	Timestamp time.Time `json:"timestamp"`
	Tool      string    `json:"tool"` // write_file | edit_file | manual | pre-revert
	Size      int       `json:"size"` // bytes, for display
}

type index struct {
	Snapshots []Snapshot `json:"snapshots"`
}

// projectDir returns (and creates) the snapshot directory for a workDir.
// Uses the first 16 hex chars of SHA-256(workDir) so filenames stay short.
func projectDir(marlinDir, workDir string) string {
	h := sha256.Sum256([]byte(workDir))
	key := fmt.Sprintf("%x", h[:8])
	dir := filepath.Join(marlinDir, "snapshots", key)
	os.MkdirAll(dir, 0755)
	return dir
}

func loadIndex(pdir string) *index {
	idx := &index{}
	data, err := os.ReadFile(filepath.Join(pdir, "index.json"))
	if err == nil {
		json.Unmarshal(data, idx)
	}
	return idx
}

func saveIndex(pdir string, idx *index) {
	data, _ := json.MarshalIndent(idx, "", "  ")
	os.WriteFile(filepath.Join(pdir, "index.json"), data, 0600)
}

// Take saves a copy of absPath before it is modified.
// If the file does not yet exist (new file), no snapshot is taken.
// tool is a label like "write_file", "edit_file", or "manual".
func Take(marlinDir, workDir, absPath, tool string) error {
	data, err := os.ReadFile(absPath)
	if os.IsNotExist(err) {
		return nil // new file — nothing to snapshot
	}
	if err != nil {
		return err
	}

	pdir := projectDir(marlinDir, workDir)
	idx := loadIndex(pdir)

	id := fmt.Sprintf("%s-%04d", time.Now().Format("2006-01-02T15-04-05"), len(idx.Snapshots))

	if err := os.WriteFile(filepath.Join(pdir, id), data, 0600); err != nil {
		return err
	}

	relPath := absPath
	if rel, err := filepath.Rel(workDir, absPath); err == nil {
		relPath = rel
	}

	idx.Snapshots = append(idx.Snapshots, Snapshot{
		ID:        id,
		File:      relPath,
		AbsFile:   absPath,
		Timestamp: time.Now(),
		Tool:      tool,
		Size:      len(data),
	})
	saveIndex(pdir, idx)
	return nil
}

// List returns all snapshots for absPath, newest first.
func List(marlinDir, workDir, absPath string) []Snapshot {
	pdir := projectDir(marlinDir, workDir)
	idx := loadIndex(pdir)

	relPath := absPath
	if rel, err := filepath.Rel(workDir, absPath); err == nil {
		relPath = rel
	}

	var matches []Snapshot
	for _, s := range idx.Snapshots {
		if s.File == relPath || s.AbsFile == absPath {
			matches = append(matches, s)
		}
	}
	sort.Slice(matches, func(i, j int) bool {
		return matches[i].Timestamp.After(matches[j].Timestamp)
	})
	return matches
}

// Restore overwrites absPath with the content of the given snapshot ID.
// It snapshots the current file first so the restore itself is also undoable.
func Restore(marlinDir, workDir, absPath, snapshotID string) error {
	// Snapshot current state before restoring so this is reversible.
	Take(marlinDir, workDir, absPath, "pre-revert")

	pdir := projectDir(marlinDir, workDir)
	data, err := os.ReadFile(filepath.Join(pdir, snapshotID))
	if err != nil {
		return fmt.Errorf("snapshot %q not found", snapshotID)
	}
	return os.WriteFile(absPath, data, 0644)
}

// HumanSize formats a byte count for display.
func HumanSize(b int) string {
	switch {
	case b < 1024:
		return fmt.Sprintf("%dB", b)
	case b < 1024*1024:
		return fmt.Sprintf("%.1fKB", float64(b)/1024)
	default:
		return fmt.Sprintf("%.1fMB", float64(b)/(1024*1024))
	}
}
