pub mod executor;
pub mod extract;

use crate::providers::{ToolDef, ToolProp};

pub fn all_tools() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "read_file".into(),
            description: "Read a file. Supply 'function' to extract only that named function \
                or method instead of the whole file — much more token-efficient for large files. \
                Supports Rust, Go, Python, and C-family languages.".into(),
            properties: vec![
                ToolProp { name: "path".into(), ty: "string".into(), description: "File path, relative to working directory or absolute.".into() },
                ToolProp { name: "function".into(), ty: "string".into(), description: "Optional: name of a function or method to extract instead of reading the whole file.".into() },
            ],
            required: vec!["path".into()],
        },
        ToolDef {
            name: "write_file".into(),
            description: "Write content to a file, creating it (and any missing parent directories) if needed. Overwrites existing content.".into(),
            properties: vec![
                ToolProp { name: "path".into(), ty: "string".into(), description: "File path.".into() },
                ToolProp { name: "content".into(), ty: "string".into(), description: "Full content to write.".into() },
            ],
            required: vec!["path".into(), "content".into()],
        },
        ToolDef {
            name: "edit_file".into(),
            description: "Replace the first occurrence of old_string with new_string in a file. Preferred over write_file for targeted edits.".into(),
            properties: vec![
                ToolProp { name: "path".into(), ty: "string".into(), description: "File path.".into() },
                ToolProp { name: "old_string".into(), ty: "string".into(), description: "Exact string to find. Must match the file content exactly.".into() },
                ToolProp { name: "new_string".into(), ty: "string".into(), description: "Replacement string.".into() },
            ],
            required: vec!["path".into(), "old_string".into(), "new_string".into()],
        },
        ToolDef {
            name: "run_command".into(),
            description: "Run a shell command in the working directory and return combined stdout/stderr.".into(),
            properties: vec![
                ToolProp { name: "command".into(), ty: "string".into(), description: "Shell command to execute.".into() },
            ],
            required: vec!["command".into()],
        },
        ToolDef {
            name: "list_directory".into(),
            description: "List files and subdirectories at a path.".into(),
            properties: vec![
                ToolProp { name: "path".into(), ty: "string".into(), description: "Directory path. Uses working directory if omitted.".into() },
            ],
            required: vec![],
        },
        ToolDef {
            name: "create_directory".into(),
            description: "Create a directory and any necessary parent directories.".into(),
            properties: vec![
                ToolProp { name: "path".into(), ty: "string".into(), description: "Directory path to create.".into() },
            ],
            required: vec!["path".into()],
        },
        ToolDef {
            name: "search_codebase".into(),
            description: "Search the project index using TF-IDF ranking. Returns the most relevant files \
                for a query with scored snippets. Use this before read_file on large codebases to find \
                the right file without reading everything. Returns an error if the index hasn't been built yet.".into(),
            properties: vec![
                ToolProp { name: "query".into(), ty: "string".into(), description: "Search terms or natural language description of what you're looking for.".into() },
                ToolProp { name: "limit".into(), ty: "string".into(), description: "Maximum number of results to return (default 5, max 20).".into() },
            ],
            required: vec!["query".into()],
        },
    ]
}
