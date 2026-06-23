// Lightweight symbol extraction: finds a named function/method in source text
// by scanning for common declaration patterns across Rust, Go, Python, C/C++, JS/TS.

pub fn extract_symbol(source: &str, name: &str, ext: &str) -> Option<String> {
    let patterns: &[&str] = match ext {
        "rs" => &[
            &format!("fn {name}("),
            &format!("fn {name}<"),
            &format!("async fn {name}("),
        ],
        "go" => &[
            &format!("func {name}("),
            &format!(") {name}("),
        ],
        "py" => &[
            &format!("def {name}("),
            &format!("async def {name}("),
        ],
        "js" | "ts" | "jsx" | "tsx" => &[
            &format!("function {name}("),
            &format!("const {name} ="),
            &format!("async function {name}("),
        ],
        "c" | "cpp" | "cc" | "h" | "hpp" => &[
            &format!("{name}("),
        ],
        _ => &[
            &format!("fn {name}("),
            &format!("func {name}("),
            &format!("def {name}("),
            &format!("function {name}("),
        ],
    };

    for pat in patterns {
        if let Some(start) = find_symbol_start(source, pat) {
            return Some(extract_from(source, start));
        }
    }
    None
}

fn find_symbol_start(source: &str, pattern: &str) -> Option<usize> {
    // Walk backwards from each match to find the start of the line
    // (catching `pub`, `async`, decorators, etc.)
    let mut search_from = 0;
    while let Some(pos) = source[search_from..].find(pattern) {
        let abs = search_from + pos;
        // Make sure it's a real definition (not inside a string or comment heuristically)
        // Find start of line
        let line_start = source[..abs].rfind('\n').map(|p| p + 1).unwrap_or(0);
        let line = &source[line_start..];
        if !line.trim_start().starts_with("//") && !line.trim_start().starts_with('#') {
            return Some(line_start);
        }
        search_from = abs + 1;
    }
    None
}

fn extract_from(source: &str, start: usize) -> String {
    let tail = &source[start..];
    let mut depth = 0i32;
    let mut in_string = false;
    let mut string_char = ' ';
    let mut prev = ' ';
    let mut end = tail.len();

    for (i, ch) in tail.char_indices() {
        if in_string {
            if ch == string_char && prev != '\\' {
                in_string = false;
            }
        } else {
            match ch {
                '"' | '\'' => { in_string = true; string_char = ch; }
                '{' => { depth += 1; }
                '}' => {
                    depth -= 1;
                    if depth == 0 && i > 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        prev = ch;
    }

    // If we never hit a {, try to grab until a blank line (Python-style)
    if depth != 0 || !tail.contains('{') {
        let lines: Vec<&str> = tail.lines().collect();
        let mut found_def = false;
        let mut result_lines = Vec::new();
        for line in &lines {
            result_lines.push(*line);
            if line.contains("def ") || line.contains("fn ") {
                found_def = true;
            }
            if found_def && line.trim().is_empty() && result_lines.len() > 2 {
                break;
            }
        }
        return result_lines.join("\n");
    }

    tail[..end].to_string()
}
