// Package index provides a pure-Go TF-IDF codebase index for semantic search.
// No CGO, no external model, no network calls.
package index

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"time"
	"unicode"
)

// ---- Public types ----

type FileEntry struct {
	Path    string             `json:"path"`     // relative to WorkDir
	Terms   map[string]float64 `json:"terms"`    // term → raw TF (count / total)
	Lines   int                `json:"lines"`
	ModTime time.Time          `json:"mod_time"`
}

type Index struct {
	BuiltAt   time.Time              `json:"built_at"`
	WorkDir   string                 `json:"work_dir"`
	FileCount int                    `json:"file_count"`
	TermCount int                    `json:"term_count"`
	IDF       map[string]float64     `json:"idf"`
	Files     map[string]*FileEntry  `json:"files"` // rel path → entry
}

type SearchResult struct {
	Path    string
	Score   float64
	Lines   int
	Snippet string // best-matching excerpt
}

type BuildStats struct {
	Files   int
	Skipped int
	Terms   int
	Elapsed time.Duration
}

// ---- Build ----

const (
	maxFileBytes = 512 * 1024 // skip files > 512 KB
	maxFiles     = 5_000
)

var indexedExts = map[string]bool{
	".go": true, ".js": true, ".ts": true, ".jsx": true, ".tsx": true,
	".py": true, ".rs": true, ".java": true, ".kt": true, ".swift": true,
	".c": true, ".cpp": true, ".cc": true, ".h": true, ".hpp": true,
	".cs": true, ".rb": true, ".php": true, ".sh": true, ".bash": true,
	".md": true, ".txt": true, ".json": true, ".yaml": true, ".yml": true,
	".toml": true, ".sql": true, ".html": true, ".css": true, ".scss": true,
	".vue": true, ".svelte": true, ".graphql": true, ".proto": true,
}

var skipDirs = map[string]bool{
	".git": true, "node_modules": true, "vendor": true, ".marlin": true,
	"dist": true, "build": true, "target": true, "__pycache__": true,
	".cache": true, ".tox": true, ".venv": true, "venv": true,
	".next": true, ".nuxt": true, "coverage": true, ".idea": true,
	".vscode": true, ".DS_Store": true,
}

// Build walks workDir and constructs a TF-IDF index.
func Build(workDir string, progress func(path string)) (*Index, BuildStats, error) {
	start := time.Now()
	idx := &Index{
		WorkDir: workDir,
		IDF:     make(map[string]float64),
		Files:   make(map[string]*FileEntry),
	}

	var skipped int
	err := filepath.WalkDir(workDir, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if d.IsDir() {
			if skipDirs[d.Name()] {
				return filepath.SkipDir
			}
			return nil
		}
		if idx.FileCount >= maxFiles {
			skipped++
			return nil
		}
		if !indexedExts[strings.ToLower(filepath.Ext(path))] {
			return nil
		}
		info, err := d.Info()
		if err != nil || info.Size() > maxFileBytes {
			skipped++
			return nil
		}

		data, err := os.ReadFile(path)
		if err != nil {
			return nil
		}

		rel, _ := filepath.Rel(workDir, path)
		if progress != nil {
			progress(rel)
		}

		tokens := tokenize(string(data))
		if len(tokens) == 0 {
			return nil
		}

		tf := computeTF(tokens)
		entry := &FileEntry{
			Path:    rel,
			Terms:   tf,
			Lines:   strings.Count(string(data), "\n") + 1,
			ModTime: info.ModTime(),
		}
		idx.Files[rel] = entry
		idx.FileCount++
		return nil
	})
	if err != nil {
		return nil, BuildStats{}, err
	}

	computeIDF(idx)
	idx.BuiltAt = time.Now()
	idx.TermCount = len(idx.IDF)

	stats := BuildStats{
		Files:   idx.FileCount,
		Skipped: skipped,
		Terms:   idx.TermCount,
		Elapsed: time.Since(start),
	}
	return idx, stats, nil
}

// UpdateFile reindexes a single file in an existing index.
// Call this after the AI writes or edits a file to keep the index fresh.
func UpdateFile(idx *Index, absPath string) {
	if idx == nil {
		return
	}
	rel, err := filepath.Rel(idx.WorkDir, absPath)
	if err != nil {
		return
	}
	data, err := os.ReadFile(absPath)
	if err != nil {
		delete(idx.Files, rel)
		computeIDF(idx)
		return
	}
	info, _ := os.Stat(absPath)
	tokens := tokenize(string(data))
	if len(tokens) == 0 {
		return
	}
	idx.Files[rel] = &FileEntry{
		Path:    rel,
		Terms:   computeTF(tokens),
		Lines:   strings.Count(string(data), "\n") + 1,
		ModTime: info.ModTime(),
	}
	computeIDF(idx)
	idx.FileCount = len(idx.Files)
	idx.TermCount = len(idx.IDF)
}

// ---- Search ----

// Search scores every file against query and returns the top n results.
func Search(idx *Index, query string, n int) []SearchResult {
	if idx == nil || len(idx.Files) == 0 {
		return nil
	}
	qTerms := tokenize(query)
	if len(qTerms) == 0 {
		return nil
	}

	type scored struct {
		path  string
		score float64
	}
	var candidates []scored

	for path, entry := range idx.Files {
		score := 0.0
		for _, t := range qTerms {
			tf := entry.Terms[t]
			idf := idx.IDF[t]
			score += tf * idf
		}
		if score > 0 {
			candidates = append(candidates, scored{path, score})
		}
	}

	sort.Slice(candidates, func(i, j int) bool {
		return candidates[i].score > candidates[j].score
	})
	if n > len(candidates) {
		n = len(candidates)
	}
	candidates = candidates[:n]

	results := make([]SearchResult, 0, len(candidates))
	for _, c := range candidates {
		entry := idx.Files[c.path]
		snippet := extractSnippet(idx.WorkDir, entry.Path, qTerms)
		results = append(results, SearchResult{
			Path:    c.path,
			Score:   math.Round(c.score*100) / 100,
			Lines:   entry.Lines,
			Snippet: snippet,
		})
	}
	return results
}

// FormatResults formats search results for display in chat or as a tool output.
func FormatResults(results []SearchResult, query string) string {
	if len(results) == 0 {
		return fmt.Sprintf("No results for %q.", query)
	}
	var sb strings.Builder
	sb.WriteString(fmt.Sprintf("Search results for %q:\n\n", query))
	for i, r := range results {
		sb.WriteString(fmt.Sprintf("%d. %s  (score %.2f, %d lines)\n", i+1, r.Path, r.Score, r.Lines))
		if r.Snippet != "" {
			for _, line := range strings.Split(r.Snippet, "\n") {
				sb.WriteString("   " + line + "\n")
			}
		}
		sb.WriteString("\n")
	}
	return strings.TrimRight(sb.String(), "\n")
}

// ---- Persistence ----

func indexDir(marlinDir, workDir string) string {
	h := sha256.Sum256([]byte(workDir))
	key := fmt.Sprintf("%x", h[:8])
	dir := filepath.Join(marlinDir, "index", key)
	os.MkdirAll(dir, 0755)
	return dir
}

func Save(marlinDir string, idx *Index) error {
	dir := indexDir(marlinDir, idx.WorkDir)
	data, err := json.Marshal(idx)
	if err != nil {
		return err
	}
	return os.WriteFile(filepath.Join(dir, "index.json"), data, 0600)
}

func Load(marlinDir, workDir string) (*Index, error) {
	dir := indexDir(marlinDir, workDir)
	data, err := os.ReadFile(filepath.Join(dir, "index.json"))
	if err != nil {
		return nil, err
	}
	var idx Index
	if err := json.Unmarshal(data, &idx); err != nil {
		return nil, err
	}
	return &idx, nil
}

// ---- Internal ----

var nonAlphaRe = regexp.MustCompile(`[^a-zA-Z0-9]+`)

// stopWords are filtered out from tokens — common English words and short
// language keywords that add noise without search value.
var stopWords = map[string]bool{
	"the": true, "a": true, "an": true, "and": true, "or": true, "but": true,
	"in": true, "on": true, "at": true, "to": true, "for": true, "of": true,
	"with": true, "by": true, "from": true, "as": true, "if": true, "is": true,
	"it": true, "its": true, "be": true, "been": true, "was": true, "are": true,
	"were": true, "have": true, "has": true, "had": true, "do": true, "did": true,
	"not": true, "no": true, "so": true, "up": true, "out": true, "this": true,
	"that": true, "then": true, "when": true, "will": true, "would": true,
	// Ubiquitous short tokens that appear everywhere
	"nil": true, "err": true, "ok": true, "id": true,
}

func tokenize(text string) []string {
	words := nonAlphaRe.Split(text, -1)
	seen := make(map[string]struct{}, 256)
	var tokens []string
	for _, w := range words {
		if len(w) < 2 {
			continue
		}
		parts := splitCamel(w)
		for _, p := range parts {
			p = strings.ToLower(p)
			if len(p) < 2 || stopWords[p] {
				continue
			}
			if _, dup := seen[p]; !dup {
				seen[p] = struct{}{}
				tokens = append(tokens, p)
			}
		}
	}
	return tokens
}

// splitCamel splits a word on CamelCase boundaries.
// Examples: validateToken → [validate Token], HTTPHandler → [HTTP Handler]
func splitCamel(s string) []string {
	runes := []rune(s)
	n := len(runes)
	if n == 0 {
		return nil
	}
	var result []string
	start := 0
	for i := 1; i < n; i++ {
		curr := unicode.IsUpper(runes[i])
		prev := unicode.IsUpper(runes[i-1])
		// lower→upper boundary: new word
		if curr && !prev {
			result = append(result, string(runes[start:i]))
			start = i
			continue
		}
		// UPPER…UPPER→lower boundary (e.g. "HTTPHandler" → "HTTP", "Handler")
		if !curr && prev && i > start+1 {
			result = append(result, string(runes[start:i-1]))
			start = i - 1
		}
	}
	result = append(result, string(runes[start:]))
	return result
}

func computeTF(tokens []string) map[string]float64 {
	counts := make(map[string]int, len(tokens))
	for _, t := range tokens {
		counts[t]++
	}
	total := float64(len(tokens))
	tf := make(map[string]float64, len(counts))
	for t, c := range counts {
		tf[t] = float64(c) / total
	}
	return tf
}

func computeIDF(idx *Index) {
	// df[term] = number of documents containing term
	df := make(map[string]int, idx.TermCount)
	for _, entry := range idx.Files {
		for t := range entry.Terms {
			df[t]++
		}
	}
	n := float64(len(idx.Files))
	idx.IDF = make(map[string]float64, len(df))
	for t, d := range df {
		// Smooth IDF: log((N+1)/(df+1)) + 1
		idx.IDF[t] = math.Log((n+1)/float64(d+1)) + 1
	}
}

const snippetContext = 3 // lines of context around the best match

func extractSnippet(workDir, relPath string, queryTerms []string) string {
	absPath := filepath.Join(workDir, relPath)
	data, err := os.ReadFile(absPath)
	if err != nil {
		return ""
	}
	lines := strings.Split(string(data), "\n")
	if len(lines) == 0 {
		return ""
	}

	// Score each line by how many query terms it contains.
	best := -1
	bestScore := 0
	for i, line := range lines {
		ll := strings.ToLower(line)
		score := 0
		for _, t := range queryTerms {
			if strings.Contains(ll, t) {
				score++
			}
		}
		if score > bestScore {
			bestScore = score
			best = i
		}
	}
	if best < 0 || bestScore == 0 {
		// No matching lines — return first few lines
		end := snippetContext * 2
		if end > len(lines) {
			end = len(lines)
		}
		return strings.Join(lines[:end], "\n")
	}

	start := best - snippetContext
	if start < 0 {
		start = 0
	}
	end := best + snippetContext + 1
	if end > len(lines) {
		end = len(lines)
	}
	return fmt.Sprintf("(lines %d–%d)\n%s", start+1, end, strings.Join(lines[start:end], "\n"))
}
