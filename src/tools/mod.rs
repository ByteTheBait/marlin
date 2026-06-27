pub mod executor;
pub mod extract;
pub mod policy;

use crate::config::AstMode;
use crate::providers::{ToolDef, ToolProp};

pub fn all_tools(ast_mode: &AstMode, skills: &[(String, String)]) -> Vec<ToolDef> {
    let mut tools = vec![
        ToolDef {
            name: "read_file".into(),
            description: {
                let base = "Read a file. Supply 'function' to extract only that named function \
                    or method instead of the whole file — much more token-efficient for large files. \
                    Supports Rust, Go, Python, and C-family languages.";
                match ast_mode {
                    AstMode::SExpr => format!(
                        "{base} [AST/SEXPR active: output is a compact S-expression AST, not raw source.]"
                    ),
                    _ => base.into(),
                }
            },
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
    ];

    // Inject skill tools when skills are loaded
    if !skills.is_empty() {
        let skill_list = skills.iter()
            .map(|(name, desc)| format!("  - {name}: {desc}"))
            .collect::<Vec<_>>()
            .join("\n");
        tools.push(ToolDef {
            name: "run_skill".into(),
            description: format!(
                "Run a pre-built Marlin skill by name. Use this when a skill matches \
                the user's request — it's faster and more reliable than writing a \
                shell command from scratch. Available skills:\n{skill_list}"
            ),
            properties: vec![
                ToolProp {
                    name: "name".into(), ty: "string".into(),
                    description: "Skill name (exactly as listed above).".into(),
                },
                ToolProp {
                    name: "query".into(), ty: "string".into(),
                    description: "Input text or search query to pass to the skill.".into(),
                },
            ],
            required: vec!["name".into()],
        });
    }

    // Inject AST harness tools only when harness mode is active
    if *ast_mode == AstMode::Harness {
        tools.push(ToolDef {
            name: "ast_skeleton".into(),
            description: "Fetch the structural skeleton of an AST JSON file — signatures and type \
                surfaces only, no function bodies. Use this first to orient yourself cheaply before \
                diving into individual nodes.".into(),
            properties: vec![
                ToolProp {
                    name: "file".into(), ty: "string".into(),
                    description: "Path to the .ast.json file.".into(),
                },
            ],
            required: vec!["file".into()],
        });

        tools.push(ToolDef {
            name: "ast_get_node".into(),
            description: "Fetch the full JSON structure of a single AST node by its node_id. Use \
                after ast_skeleton to inspect the body of a specific function, type, or statement.".into(),
            properties: vec![
                ToolProp {
                    name: "file".into(), ty: "string".into(),
                    description: "Path to the .ast.json file.".into(),
                },
                ToolProp {
                    name: "node_id".into(), ty: "string".into(),
                    description: "The node identifier from the skeleton output.".into(),
                },
            ],
            required: vec!["file".into(), "node_id".into()],
        });

        tools.push(ToolDef {
            name: "ast_mutate".into(),
            description: "Mutate an AST node using a structural JSON directive, then automatically \
                recompile and optimize the source file. Supported operations: \
                'str-replace' (requires old_json, new_json), \
                'append-stmt' (requires statement_json), \
                'insert-before' (requires index, statement_json). \
                Always provide 'lang' and 'source_file' so the source can be regenerated.".into(),
            properties: vec![
                ToolProp {
                    name: "file".into(), ty: "string".into(),
                    description: "Path to the .ast.json file.".into(),
                },
                ToolProp {
                    name: "node_id".into(), ty: "string".into(),
                    description: "Target node identifier.".into(),
                },
                ToolProp {
                    name: "operation".into(), ty: "string".into(),
                    description: "One of: str-replace, append-stmt, insert-before.".into(),
                },
                ToolProp {
                    name: "old_json".into(), ty: "string".into(),
                    description: "JSON value to replace (str-replace only).".into(),
                },
                ToolProp {
                    name: "new_json".into(), ty: "string".into(),
                    description: "Replacement JSON value (str-replace only).".into(),
                },
                ToolProp {
                    name: "statement_json".into(), ty: "string".into(),
                    description: "Statement JSON to append or insert (append-stmt / insert-before).".into(),
                },
                ToolProp {
                    name: "index".into(), ty: "string".into(),
                    description: "Insertion index (insert-before only).".into(),
                },
                ToolProp {
                    name: "lang".into(), ty: "string".into(),
                    description: "Target language for recompilation (e.g. rust, go, python).".into(),
                },
                ToolProp {
                    name: "source_file".into(), ty: "string".into(),
                    description: "Output source file path to regenerate after mutation.".into(),
                },
            ],
            required: vec!["file".into(), "node_id".into(), "operation".into()],
        });
    }

    tools
}
