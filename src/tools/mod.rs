pub mod executor;
pub mod external;
pub mod extract;
pub mod policy;

use crate::config::AstMode;
use crate::mcp;
use crate::providers::{ToolDef, ToolProp};

pub fn all_tools(
    ast_mode: &AstMode,
    skills: &[(String, String)],
    external: &[external::ExternalTool],
    subagents_enabled: bool,
    mcp_tools: &[(String, mcp::client::McpTool)],
) -> Vec<ToolDef> {
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
            name: "notebook_edit".into(),
            description: "Replace, insert, or delete a single cell in a Jupyter notebook (.ipynb file).".into(),
            properties: vec![
                ToolProp { name: "path".into(), ty: "string".into(), description: "Path to the .ipynb file, relative to working directory or absolute.".into() },
                ToolProp { name: "cell_id".into(), ty: "string".into(), description: "id of the target cell. Required for edit_mode=replace and edit_mode=delete. For edit_mode=insert, the new cell is inserted after this cell, or at the start of the notebook if omitted.".into() },
                ToolProp { name: "cell_type".into(), ty: "string".into(), description: "One of: code, markdown. Required for edit_mode=insert; if omitted for replace, the existing cell's type is kept.".into() },
                ToolProp { name: "edit_mode".into(), ty: "string".into(), description: "One of: replace, insert, delete. Defaults to replace.".into() },
                ToolProp { name: "new_source".into(), ty: "string".into(), description: "New source for the cell. Not used for edit_mode=delete.".into() },
            ],
            required: vec!["path".into(), "new_source".into()],
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
            description: "TF-IDF search over the project index. Use before read_file on large codebases \
                to find the right file without reading everything. Errors if the index isn't built yet.".into(),
            properties: vec![
                ToolProp { name: "query".into(), ty: "string".into(), description: "Search terms or natural language description of what you're looking for.".into() },
                ToolProp { name: "limit".into(), ty: "string".into(), description: "Maximum number of results to return (default 5, max 20).".into() },
            ],
            required: vec!["query".into()],
        },
    ];

    // Inject skill tools when skills are loaded. `skills` is expected to already
    // be narrowed to this turn's trigger-matched subset (falling back to bare
    // names when nothing matched) — see Engine::skill_tool_list — so this list
    // stays bounded instead of scaling with the number of installed skills.
    if !skills.is_empty() {
        let skill_list = skills.iter()
            .map(|(name, desc)| if desc.is_empty() { format!("  - {name}") } else { format!("  - {name}: {desc}") })
            .collect::<Vec<_>>()
            .join("\n");
        let delegation_note = if subagents_enabled {
            "Calling this delegates the work to a subagent — a separate agent instance with \
             its own tools that completes the task independently and reports back one final \
             summary. You will NOT see its intermediate steps or raw tool output, only that \
             summary; treat it as a trustworthy report rather than something to re-verify \
             yourself unless the summary itself looks wrong."
        } else {
            "Runs directly in this conversation (subagent delegation is currently off) — you \
             get the skill's raw output back as the tool result."
        };
        tools.push(ToolDef {
            name: "run_skill".into(),
            description: format!(
                "Run a pre-built Marlin skill by name. Use this when a skill matches \
                the user's request — it's faster and more reliable than writing a \
                shell command from scratch. {delegation_note} Candidate skills for this turn:\n{skill_list}"
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
            description: "Mutate an AST node via a structural JSON directive (str-replace, append-stmt, \
                insert-before — see each property below), then recompile and optimize the source file. \
                Provide 'lang' and 'source_file' to regenerate the source.".into(),
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

    // Append user-defined external tools from ~/.marlin/tools/*.toml.
    for et in external {
        tools.push(et.to_tool_def());
    }

    // Append tools discovered from configured MCP servers (~/.marlin/mcp/*.json).
    for (server_name, tool) in mcp_tools {
        tools.push(mcp::tool_def(server_name, tool));
    }

    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_skill_desc(tools: &[ToolDef]) -> &str {
        &tools.iter().find(|t| t.name == "run_skill").unwrap().description
    }

    #[test]
    fn run_skill_description_names_only_when_bounded_fallback() {
        // Engine::skill_tool_list falls back to (name, "") pairs when nothing
        // trigger-matches the current turn — proves the description scales
        // with skill *names*, not full descriptions, however many are installed.
        let ten_names: Vec<(String, String)> = (0..10)
            .map(|i| (format!("skill_{i}"), String::new()))
            .collect();
        let tools = all_tools(&AstMode::Off, &ten_names, &[], true, &[]);
        let desc = run_skill_desc(&tools);
        for i in 0..10 {
            assert!(desc.contains(&format!("skill_{i}")), "missing skill_{i} in: {desc}");
        }
        // No description text (colon-separated) should appear for the fallback case.
        assert!(!desc.contains(':') || desc.matches(':').count() <= 1, "unexpected description text: {desc}");
    }

    #[test]
    fn run_skill_description_bounded_by_matched_subset() {
        // A large skill count with only a couple of matches (the normal
        // Engine::skill_tool_list path) keeps the description small regardless
        // of how many skills are installed overall.
        let matched = vec![("relevant_skill".to_string(), "does the relevant thing".to_string())];
        let tools = all_tools(&AstMode::Off, &matched, &[], true, &[]);
        let desc = run_skill_desc(&tools);
        assert!(desc.contains("relevant_skill: does the relevant thing"));
        // Bounded by the fixed delegation-note overhead, not skill count — a
        // regression here would mean the description started scaling with
        // something other than the (already-narrowed) matched skill list.
        assert!(desc.len() < 700, "run_skill description unexpectedly large: {} chars", desc.len());
    }

    #[test]
    fn run_skill_description_reflects_subagent_toggle() {
        let matched = vec![("s".to_string(), "d".to_string())];
        let on = all_tools(&AstMode::Off, &matched, &[], true, &[]);
        let off = all_tools(&AstMode::Off, &matched, &[], false, &[]);
        assert!(run_skill_desc(&on).contains("delegates the work to a subagent"));
        assert!(run_skill_desc(&off).contains("Runs directly in this conversation"));
    }
}
