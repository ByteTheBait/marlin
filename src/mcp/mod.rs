//! Model Context Protocol client — lets marlin talk to any MCP server (stdio
//! transport) as a source of tools, alongside the built-in tools and
//! `~/.marlin/tools/*.toml` external tools. Hand-rolled JSON-RPC over stdio
//! (no new dependency — just tokio + serde_json, both already in the tree)
//! rather than pulling in an SDK crate, since the subset of MCP actually
//! needed here (initialize handshake, tools/list, tools/call) is small.
//!
//! Scope: tools only (no resources/prompts), stdio transport only (no
//! SSE/HTTP servers). MCP tool calls bypass marlin's shell/path preflight
//! funnel entirely — they have structured typed arguments, not a shell
//! string or file path, so that funnel doesn't apply to them by
//! construction. Trust model matches `tools::external::ExternalTool`: a
//! server the user explicitly configured in `~/.marlin/mcp/*.json` is
//! trusted, same as a hand-written external tool.

pub mod client;

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

pub fn mcp_dir(marlin_dir: &Path) -> PathBuf {
    let d = marlin_dir.join("mcp");
    let _ = std::fs::create_dir_all(&d);
    d
}

pub fn load_all(marlin_dir: &Path) -> Vec<McpServerConfig> {
    let dir = mcp_dir(marlin_dir);
    let mut servers = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return servers;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(data) = std::fs::read_to_string(&path) {
            match serde_json::from_str::<McpServerConfig>(&data) {
                Ok(cfg) => servers.push(cfg),
                Err(e) => eprintln!("mcp: parse error in {:?}: {e}", path.file_name()),
            }
        }
    }
    servers.sort_by(|a, b| a.name.cmp(&b.name));
    servers
}

/// Converts one MCP tool into a marlin `ToolDef`, namespaced as
/// `mcp__{server_name}__{tool_name}` (same convention Claude Code itself uses
/// for MCP tools) so tools from different servers can't collide, and so
/// `Engine::execute_tools` can route a call back to the right server by
/// parsing the name. Only the JSON Schema's top-level `properties`/`required`
/// are unpacked into `ToolProp`s — nested/array parameter schemas aren't
/// representable in marlin's flat tool-property model, so a server relying on
/// those will show a caller-facing schema that's simpler than what it
/// actually validates against.
pub fn tool_def(server_name: &str, tool: &client::McpTool) -> crate::providers::ToolDef {
    use crate::providers::{ToolDef, ToolProp};

    let required: Vec<String> = tool
        .input_schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let properties = tool
        .input_schema
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|obj| {
            obj.iter()
                .map(|(name, schema)| ToolProp {
                    name: name.clone(),
                    ty: schema
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("string")
                        .to_string(),
                    description: schema
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                })
                .collect()
        })
        .unwrap_or_default();

    ToolDef {
        name: format!("mcp__{server_name}__{}", tool.name),
        description: if tool.description.is_empty() {
            format!("MCP tool '{}' from server '{server_name}'.", tool.name)
        } else {
            tool.description.clone()
        },
        properties,
        required,
    }
}

/// Splits a `mcp__{server}__{tool}` tool name back into its parts — the
/// inverse of `tool_def`'s naming. `None` if `name` isn't MCP-namespaced.
pub fn parse_tool_name(name: &str) -> Option<(&str, &str)> {
    let rest = name.strip_prefix("mcp__")?;
    rest.split_once("__")
}

/// Write a starter config for `/mcp new <name> <command> [args...]`.
pub fn save_template(
    marlin_dir: &Path,
    name: &str,
    command: &str,
    args: Vec<String>,
) -> Result<PathBuf> {
    let cfg = McpServerConfig {
        name: name.to_string(),
        command: command.to_string(),
        args,
        env: Default::default(),
    };
    let filename = format!("{}.json", name.replace([' ', '/'], "_").to_lowercase());
    let path = mcp_dir(marlin_dir).join(filename);
    std::fs::write(&path, serde_json::to_string_pretty(&cfg)?)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_def_unpacks_flat_schema_into_properties() {
        let tool = client::McpTool {
            name: "search".into(),
            description: "Search things".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "search terms" },
                    "limit": { "type": "number" },
                },
                "required": ["query"],
            }),
        };
        let def = tool_def("myserver", &tool);
        assert_eq!(def.name, "mcp__myserver__search");
        assert_eq!(def.description, "Search things");
        assert_eq!(def.required, vec!["query".to_string()]);
        assert_eq!(def.properties.len(), 2);
        let query_prop = def.properties.iter().find(|p| p.name == "query").unwrap();
        assert_eq!(query_prop.ty, "string");
        assert_eq!(query_prop.description, "search terms");
    }

    #[test]
    fn tool_def_falls_back_to_a_generated_description_when_empty() {
        let tool = client::McpTool {
            name: "noop".into(),
            description: String::new(),
            input_schema: json!({ "type": "object" }),
        };
        let def = tool_def("srv", &tool);
        assert!(def.description.contains("noop"));
        assert!(def.description.contains("srv"));
        assert!(def.properties.is_empty());
        assert!(def.required.is_empty());
    }

    #[test]
    fn parse_tool_name_round_trips_with_tool_def_naming() {
        let tool = client::McpTool {
            name: "search".into(),
            description: String::new(),
            input_schema: json!({}),
        };
        let def = tool_def("myserver", &tool);
        assert_eq!(parse_tool_name(&def.name), Some(("myserver", "search")));
    }

    #[test]
    fn parse_tool_name_rejects_non_mcp_names() {
        assert_eq!(parse_tool_name("read_file"), None);
        assert_eq!(parse_tool_name("mcp__onlyone"), None);
    }

    #[test]
    fn loads_valid_configs_and_skips_unparsable_ones() {
        let dir = std::env::temp_dir().join("marlin_mcp_test_load_all");
        let _ = std::fs::remove_dir_all(&dir);
        let marlin_dir = dir.join("marlin_home");
        std::fs::create_dir_all(mcp_dir(&marlin_dir)).unwrap();

        std::fs::write(
            mcp_dir(&marlin_dir).join("good.json"),
            r#"{"name":"good","command":"echo","args":["hi"]}"#,
        )
        .unwrap();
        std::fs::write(mcp_dir(&marlin_dir).join("bad.json"), "not json").unwrap();
        std::fs::write(mcp_dir(&marlin_dir).join("ignored.txt"), "irrelevant").unwrap();

        let servers = load_all(&marlin_dir);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "good");
        assert_eq!(servers[0].args, vec!["hi".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_template_round_trips() {
        let dir = std::env::temp_dir().join("marlin_mcp_test_save_template");
        let _ = std::fs::remove_dir_all(&dir);
        let marlin_dir = dir.join("marlin_home");

        let path = save_template(
            &marlin_dir,
            "My Server",
            "npx",
            vec!["-y".into(), "some-pkg".into()],
        )
        .unwrap();
        assert!(path.exists());
        let servers = load_all(&marlin_dir);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "My Server");
        assert_eq!(servers[0].command, "npx");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
