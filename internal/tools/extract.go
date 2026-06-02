package tools

import (
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"strings"
)

// ExtractSymbol pulls a single function or method out of src.
// For .go files it uses go/ast for precision; for everything else it uses a
// brace-counting line scanner that handles C, JS/TS, Rust, Java, Swift, etc.
// Python uses indent-tracking instead of braces.
func ExtractSymbol(src []byte, symbolName, ext string) (string, error) {
	switch ext {
	case "go":
		return extractGoSymbol(src, symbolName)
	case "py":
		return extractPythonSymbol(src, symbolName)
	default:
		return extractByBraces(src, symbolName)
	}
}

// extractGoSymbol uses go/ast to find a named function or method declaration.
func extractGoSymbol(src []byte, name string) (string, error) {
	fset := token.NewFileSet()
	f, err := parser.ParseFile(fset, "", src, parser.ParseComments)
	if err != nil {
		// Fallback to brace scanner if the file doesn't parse cleanly.
		return extractByBraces(src, name)
	}
	for _, decl := range f.Decls {
		fd, ok := decl.(*ast.FuncDecl)
		if !ok || fd.Name.Name != name {
			continue
		}
		startLine := fset.Position(fd.Pos()).Line - 1
		endLine := fset.Position(fd.End()).Line
		lines := strings.Split(string(src), "\n")
		if endLine > len(lines) {
			endLine = len(lines)
		}
		// Include any doc comment that immediately precedes the function.
		commentStart := startLine
		if fd.Doc != nil {
			commentStart = fset.Position(fd.Doc.Pos()).Line - 1
		}
		return strings.Join(lines[commentStart:endLine], "\n"), nil
	}
	return "", fmt.Errorf("symbol %q not found in Go file", name)
}

// extractByBraces finds a symbol in C-family / curly-brace languages by:
// 1. Scanning for a line that looks like a declaration for symbolName.
// 2. Counting braces from that point until they balance back to zero.
func extractByBraces(src []byte, name string) (string, error) {
	lines := strings.Split(string(src), "\n")
	startLine := -1

	// Patterns that indicate a function/method declaration.
	// We look for the symbol name followed by '(' on the same line.
	needle := name + "("
	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if strings.Contains(trimmed, needle) {
			// Must look like a declaration, not a call inside another function.
			// Heuristic: line starts with a keyword or a type/visibility modifier,
			// or the function name appears at low indentation.
			lc := strings.ToLower(trimmed)
			if strings.HasPrefix(lc, "function ") ||
				strings.HasPrefix(lc, "fn ") ||
				strings.HasPrefix(lc, "pub fn ") ||
				strings.HasPrefix(lc, "func ") ||
				strings.HasPrefix(lc, "def ") ||
				strings.HasPrefix(lc, "async function ") ||
				strings.HasPrefix(lc, "export function ") ||
				strings.HasPrefix(lc, "export default function ") ||
				strings.HasPrefix(lc, "public ") ||
				strings.HasPrefix(lc, "private ") ||
				strings.HasPrefix(lc, "protected ") ||
				strings.HasPrefix(lc, "static ") ||
				strings.HasPrefix(lc, "override ") ||
				strings.HasPrefix(trimmed, name+"(") {
				startLine = i
				break
			}
		}
	}

	if startLine < 0 {
		return "", fmt.Errorf("symbol %q not found", name)
	}

	// Walk forward counting braces until they balance.
	depth := 0
	foundOpen := false
	for i := startLine; i < len(lines); i++ {
		for _, ch := range lines[i] {
			if ch == '{' {
				depth++
				foundOpen = true
			} else if ch == '}' {
				depth--
			}
		}
		if foundOpen && depth == 0 {
			return strings.Join(lines[startLine:i+1], "\n"), nil
		}
	}

	// If no braces found at all (e.g. single-expression arrow functions), return to EOF.
	return strings.Join(lines[startLine:], "\n"), nil
}

// extractPythonSymbol finds a Python def/class by name and collects its
// body by tracking indentation depth.
func extractPythonSymbol(src []byte, name string) (string, error) {
	lines := strings.Split(string(src), "\n")
	startLine := -1
	baseIndent := 0

	for i, line := range lines {
		trimmed := strings.TrimSpace(line)
		if (strings.HasPrefix(trimmed, "def "+name+"(") ||
			strings.HasPrefix(trimmed, "async def "+name+"(") ||
			strings.HasPrefix(trimmed, "class "+name+":") ||
			strings.HasPrefix(trimmed, "class "+name+"(")) {
			startLine = i
			baseIndent = len(line) - len(strings.TrimLeft(line, " \t"))
			break
		}
	}
	if startLine < 0 {
		return "", fmt.Errorf("symbol %q not found in Python file", name)
	}

	end := len(lines)
	for i := startLine + 1; i < len(lines); i++ {
		line := lines[i]
		if strings.TrimSpace(line) == "" {
			continue
		}
		indent := len(line) - len(strings.TrimLeft(line, " \t"))
		if indent <= baseIndent {
			end = i
			break
		}
	}
	return strings.Join(lines[startLine:end], "\n"), nil
}
