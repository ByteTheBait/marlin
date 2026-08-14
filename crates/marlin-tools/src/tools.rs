use marlin_config::config::AstMode;
use marlin_mcp::mcp;
use marlin_providers::{ToolDef, ToolProp};

pub fn all_tools(
    ast_mode: &AstMode,
    skills: &[(String, String)],
    external: &[crate::external::ExternalTool],
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
            name: "multi_edit".into(),
            description: "Apply several old_string→new_string replacements to one file in a single \
                call, in order. Each edit runs against the file after the previous edits in the \
                same call, so later edits can build on earlier ones. Use for a batch of related \
                edits to one file instead of repeated edit_file calls. An empty old_string is not \
                allowed (use edit_file for inserts).".into(),
            properties: vec![
                ToolProp { name: "path".into(), ty: "string".into(), description: "File path.".into() },
                ToolProp { name: "edits".into(), ty: "string".into(), description: "JSON array of {old_string, new_string} in apply order, e.g. [{\"old_string\":\"a\",\"new_string\":\"b\"}]".into() },
            ],
            required: vec!["path".into(), "edits".into()],
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
            description: "Run a shell command in the working directory and return combined stdout/stderr. \
                If the command exceeds 'timeout' seconds it is killed and partial output is returned. \
                For long-running work (a dev server, a watch build, a long test) prefer bg_start so the \
                process keeps running while you continue working — poll it later with bg_status/bg_log.".into(),
            properties: vec![
                ToolProp { name: "command".into(), ty: "string".into(), description: "Shell command to execute.".into() },
                ToolProp { name: "timeout".into(), ty: "string".into(), description: "Optional timeout in seconds (default 120). The command is killed if it runs longer.".into() },
            ],
            required: vec!["command".into()],
        },
        ToolDef {
            name: "bg_start".into(),
            description: "Start a long-running process in the background (dev server, watch build, long test) \
                and return immediately with a process id — it keeps running while you continue working. \
                Poll it later with bg_status / bg_log, and stop it with bg_kill.".into(),
            properties: vec![
                ToolProp { name: "command".into(), ty: "string".into(), description: "Shell command to run in the background.".into() },
            ],
            required: vec!["command".into()],
        },
        ToolDef {
            name: "bg_status".into(),
            description: "Report the status of background process(es): running or exited, exit code, \
                elapsed time, and output size. With no id, lists all background processes. Use this \
                to check whether a bg_start'd server/watch/test is still alive.".into(),
            properties: vec![
                ToolProp { name: "id".into(), ty: "string".into(), description: "Optional background process id. Omit to list all.".into() },
            ],
            required: vec![],
        },
        ToolDef {
            name: "bg_log".into(),
            description: "Read the new stdout+stderr that a background process has produced since the \
                last bg_log call. Poll this after bg_status shows the process is running to see its \
                output (logs, errors, readiness messages).".into(),
            properties: vec![
                ToolProp { name: "id".into(), ty: "string".into(), description: "Background process id.".into() },
            ],
            required: vec!["id".into()],
        },
        ToolDef {
            name: "bg_kill".into(),
            description: "Terminate a background process started with bg_start (SIGTERM then SIGKILL on \
                unix). Use this to stop a dev server or watch build you no longer need.".into(),
            properties: vec![
                ToolProp { name: "id".into(), ty: "string".into(), description: "Background process id to terminate.".into() },
            ],
            required: vec!["id".into()],
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
        ToolDef {
            name: "grep".into(),
            description: "Search file contents with a regular expression, returning matching lines with \
                line numbers and context. Use this to find where a symbol is used, a string appears, or \
                a pattern occurs across the project — faster and more precise than search_codebase for \
                exact/regex matches. Skips binary files and common junk directories (node_modules, \
                target, .git, etc.).".into(),
            properties: vec![
                ToolProp { name: "pattern".into(), ty: "string".into(), description: "Regular expression to search for (Rust regex syntax).".into() },
                ToolProp { name: "path".into(), ty: "string".into(), description: "File or directory to search. Defaults to the working directory.".into() },
                ToolProp { name: "glob".into(), ty: "string".into(), description: "Optional file glob to filter which files are searched (e.g. '*.rs' or 'src/**/*.ts').".into() },
                ToolProp { name: "context".into(), ty: "string".into(), description: "Number of lines of context before/after each match (default 0).".into() },
                ToolProp { name: "limit".into(), ty: "string".into(), description: "Maximum number of matches to return (default 50, max 200).".into() },
            ],
            required: vec!["pattern".into()],
        },
        ToolDef {
            name: "glob".into(),
            description: "Find files and directories by path pattern (glob). Use this to locate files by \
                name or path shape — e.g. '**/*.test.ts', 'src/**/mod.rs', '*.toml'. Returns matching \
                paths relative to the working directory. Skips common junk directories (node_modules, \
                target, .git, etc.).".into(),
            properties: vec![
                ToolProp { name: "pattern".into(), ty: "string".into(), description: "Glob pattern to match (e.g. '**/*.rs' or 'src/**/*.test.ts').".into() },
                ToolProp { name: "limit".into(), ty: "string".into(), description: "Maximum number of results to return (default 100, max 500).".into() },
            ],
            required: vec!["pattern".into()],
        },
        ToolDef {
            name: "search_symbols".into(),
            description: "Find which file *defines* a symbol (function, type, class, const, enum, trait). \
                Returns the file path and a snippet of the definition line. Use this when you know a \
                function or type name and want to jump to its definition, instead of grepping broadly.".into(),
            properties: vec![
                ToolProp { name: "symbol".into(), ty: "string".into(), description: "The symbol name to find (e.g. 'parse_args' or 'Config').".into() },
                ToolProp { name: "limit".into(), ty: "string".into(), description: "Maximum number of results to return (default 5, max 20).".into() },
            ],
            required: vec!["symbol".into()],
        },
        ToolDef {
            name: "mark_complete".into(),
            description: "Call this — alone, with no other tool calls in the same turn — when \
                the goal is fully achieved (every requested change actually made, not just \
                described) or you're permanently blocked. This is the only way to end the turn \
                as done; plain text alone is not recognized as completion.".into(),
            properties: vec![
                ToolProp { name: "summary".into(), ty: "string".into(), description: "One or two sentences: what was accomplished, or if blocked, what's blocking you and what you need from the user.".into() },
            ],
            required: vec!["summary".into()],
        },
        ToolDef {
            name: "ask_user".into(),
            description: "Ask the user a question and wait for their typed answer. Use this when \
                you need a decision, clarification, a preference, or information only the user \
                has (e.g. which option they want, whether to proceed a certain way, a value \
                you can't infer). The user's reply is returned as the tool result. Phrase the \
                question clearly and self-contained so they can answer without extra context. \
                Do not call this when the answer is already available or inferable.".into(),
            properties: vec![
                ToolProp { name: "question".into(), ty: "string".into(), description: "The question to ask the user. Be specific and single-focus.".into() },
            ],
            required: vec!["question".into()],
        },
    ];

    // Inject skill tools when skills are loaded. `skills` is expected to already
    // be narrowed to this turn's trigger-matched subset (falling back to bare
    // names when nothing matched) — see Engine::skill_tool_list — so this list
    // stays bounded instead of scaling with the number of installed skills.
    if !skills.is_empty() {
        let skill_list = skills
            .iter()
            .map(|(name, desc)| {
                if desc.is_empty() {
                    format!("  - {name}")
                } else {
                    format!("  - {name}: {desc}")
                }
            })
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
                after ast_skeleton to inspect the body of a specific function, type, or statement."
                .into(),
            properties: vec![
                ToolProp {
                    name: "file".into(),
                    ty: "string".into(),
                    description: "Path to the .ast.json file.".into(),
                },
                ToolProp {
                    name: "node_id".into(),
                    ty: "string".into(),
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
        &tools
            .iter()
            .find(|t| t.name == "run_skill")
            .unwrap()
            .description
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
            assert!(
                desc.contains(&format!("skill_{i}")),
                "missing skill_{i} in: {desc}"
            );
        }
        // No description text (colon-separated) should appear for the fallback case.
        assert!(
            !desc.contains(':') || desc.matches(':').count() <= 1,
            "unexpected description text: {desc}"
        );
    }

    #[test]
    fn run_skill_description_bounded_by_matched_subset() {
        // A large skill count with only a couple of matches (the normal
        // Engine::skill_tool_list path) keeps the description small regardless
        // of how many skills are installed overall.
        let matched = vec![(
            "relevant_skill".to_string(),
            "does the relevant thing".to_string(),
        )];
        let tools = all_tools(&AstMode::Off, &matched, &[], true, &[]);
        let desc = run_skill_desc(&tools);
        assert!(desc.contains("relevant_skill: does the relevant thing"));
        // Bounded by the fixed delegation-note overhead, not skill count — a
        // regression here would mean the description started scaling with
        // something other than the (already-narrowed) matched skill list.
        assert!(
            desc.len() < 700,
            "run_skill description unexpectedly large: {} chars",
            desc.len()
        );
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
