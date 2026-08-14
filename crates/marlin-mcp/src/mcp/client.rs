use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};

use super::McpServerConfig;

const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Requests waiting on a response, keyed by JSON-RPC id — the reader task
/// resolves each sender with `Ok(result)` or `Err(message)` when a matching
/// response line arrives.
type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    /// Raw JSON Schema `inputSchema` object from `tools/list` — only the
    /// top-level `properties`/`required` are used (see `ToolDef` conversion
    /// in `Engine::mcp_tool_defs`); nested/array schemas aren't unpacked.
    pub input_schema: Value,
}

/// One live connection to an MCP server (stdio transport). A background task
/// reads response lines from the server's stdout and dispatches them to
/// whichever `call()` is waiting on that request id via a oneshot channel —
/// this lets multiple calls be in flight concurrently on the same
/// connection, matching marlin's own parallel tool-call batching.
pub struct McpClient {
    pub server_name: String,
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: PendingMap,
    next_id: AtomicU64,
    // Held only to keep the child process alive for the client's lifetime —
    // dropping it kills the server.
    _child: Child,
}

impl McpClient {
    /// Spawn the server process and perform the MCP initialize handshake.
    pub async fn spawn(cfg: &McpServerConfig) -> Result<Self> {
        let mut command = Command::new(&cfg.command);
        command
            .args(&cfg.args)
            .envs(&cfg.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // MCP servers may write logs to stderr — discard rather than
            // inherit, so a chatty server can't interleave with marlin's own
            // terminal output (which the TUI owns via raw mode).
            .stderr(Stdio::null());

        let mut child = command.spawn().map_err(|e| {
            anyhow!(
                "failed to spawn MCP server '{}' ({}): {e}",
                cfg.name,
                cfg.command
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("MCP server '{}': no stdin", cfg.name))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("MCP server '{}': no stdout", cfg.name))?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        spawn_reader(stdout, pending.clone());

        let client = Self {
            server_name: cfg.name.clone(),
            stdin: Mutex::new(stdin),
            pending,
            next_id: AtomicU64::new(1),
            _child: child,
        };

        client
            .call(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": { "name": "marlin", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await
            .map_err(|e| anyhow!("MCP server '{}' failed to initialize: {e}", cfg.name))?;

        client
            .notify("notifications/initialized", json!({}))
            .await?;

        Ok(client)
    }

    async fn write_line(&self, value: &Value) -> Result<()> {
        let mut line = serde_json::to_string(value)?;
        line.push('\n');
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await?;
        Ok(())
    }

    /// Send a request and await its matching response (by id), with a timeout.
    pub async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);

        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.write_line(&req).await {
            self.pending.lock().await.remove(&id);
            return Err(e);
        }

        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(Ok(result))) => Ok(result),
            Ok(Ok(Err(msg))) => Err(anyhow!("mcp '{}': {msg}", self.server_name)),
            Ok(Err(_)) => Err(anyhow!(
                "mcp '{}': connection closed before a response arrived",
                self.server_name
            )),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(anyhow!(
                    "mcp '{}': timed out after {}s waiting for a response to {method}",
                    self.server_name,
                    CALL_TIMEOUT.as_secs()
                ))
            }
        }
    }

    /// Fire-and-forget — no id, no response expected (per JSON-RPC 2.0 notifications).
    async fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.write_line(&json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let result = self.call("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(tools
            .into_iter()
            .filter_map(|t| {
                Some(McpTool {
                    name: t.get("name")?.as_str()?.to_string(),
                    description: t
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object" })),
                })
            })
            .collect())
    }

    /// Returns `(text, is_error)` — MCP's own `isError` flag, not a transport
    /// failure (a transport failure is an `Err` from this function).
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<(String, bool)> {
        let result = self
            .call(
                "tools/call",
                json!({ "name": name, "arguments": arguments }),
            )
            .await?;
        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let text = result
            .get("content")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(no output)".to_string());
        Ok((text, is_error))
    }
}

fn spawn_reader(stdout: tokio::process::ChildStdout, pending: PendingMap) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = match lines.next_line().await {
                Ok(Some(l)) => l,
                _ => break, // EOF or read error — server exited or pipe broke.
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let Ok(msg) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            // Notifications (server → client, no reply expected) and requests
            // the server might send us (unsupported in this client) both lack
            // a numeric `id` we're waiting on — ignore them rather than error.
            let Some(id) = msg.get("id").and_then(|v| v.as_u64()) else {
                continue;
            };

            let mut pend = pending.lock().await;
            let Some(tx) = pend.remove(&id) else { continue };
            if let Some(err) = msg.get("error") {
                let text = err
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("MCP error")
                    .to_string();
                let _ = tx.send(Err(text));
            } else {
                let _ = tx.send(Ok(msg.get("result").cloned().unwrap_or(Value::Null)));
            }
        }
        // Server exited (or its pipe broke) — wake up anything still waiting
        // instead of leaving those calls to hang until their own timeout.
        let mut pend = pending.lock().await;
        for (_, tx) in pend.drain() {
            let _ = tx.send(Err("MCP server process exited".to_string()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Writes a throwaway Python MCP-server fixture and returns its path, or
    /// `None` if python3 isn't on PATH — tests using it skip cleanly rather
    /// than failing in environments without Python.
    fn python_fixture() -> Option<std::path::PathBuf> {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return None;
        }
        let path = std::env::temp_dir().join("marlin_mcp_test_fixture_server.py");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, r#"
import sys, json

def send(msg):
    sys.stdout.write(json.dumps(msg) + "\n")
    sys.stdout.flush()

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    req = json.loads(line)
    method = req.get("method")
    if method == "initialize":
        send({{"jsonrpc": "2.0", "id": req["id"], "result": {{"protocolVersion": "2024-11-05"}}}})
    elif method == "notifications/initialized":
        pass
    elif method == "tools/list":
        send({{"jsonrpc": "2.0", "id": req["id"], "result": {{"tools": [
            {{"name": "echo", "description": "Echoes input back",
              "inputSchema": {{"type": "object", "properties": {{"text": {{"type": "string"}}}}, "required": ["text"]}}}}
        ]}}}})
    elif method == "tools/call":
        args = req.get("params", {{}}).get("arguments", {{}})
        name = req.get("params", {{}}).get("name")
        if name == "echo":
            send({{"jsonrpc": "2.0", "id": req["id"], "result": {{
                "content": [{{"type": "text", "text": "echo: " + str(args.get("text", ""))}}],
                "isError": False,
            }}}})
        else:
            send({{"jsonrpc": "2.0", "id": req["id"], "result": {{
                "content": [{{"type": "text", "text": "unknown tool"}}],
                "isError": True,
            }}}})
"#).unwrap();
        Some(path)
    }

    #[tokio::test]
    async fn full_handshake_list_and_call_round_trip() {
        let Some(script) = python_fixture() else {
            eprintln!("skipping: python3 not found on PATH");
            return;
        };

        let cfg = McpServerConfig {
            name: "fixture".into(),
            command: "python3".into(),
            args: vec![script.to_string_lossy().to_string()],
            env: Default::default(),
        };

        let client = McpClient::spawn(&cfg)
            .await
            .expect("spawn+initialize should succeed");

        let tools = client
            .list_tools()
            .await
            .expect("tools/list should succeed");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(
            tools[0]
                .input_schema
                .get("required")
                .and_then(|v| v.as_array())
                .map(|a| a.len()),
            Some(1)
        );

        let (text, is_error) = client
            .call_tool("echo", json!({ "text": "hello" }))
            .await
            .unwrap();
        assert_eq!(text, "echo: hello");
        assert!(!is_error);

        let (_, is_error) = client.call_tool("nonexistent", json!({})).await.unwrap();
        assert!(is_error);
    }

    #[tokio::test]
    async fn concurrent_calls_on_one_connection_resolve_to_the_right_caller() {
        let Some(script) = python_fixture() else {
            eprintln!("skipping: python3 not found on PATH");
            return;
        };

        let cfg = McpServerConfig {
            name: "fixture".into(),
            command: "python3".into(),
            args: vec![script.to_string_lossy().to_string()],
            env: Default::default(),
        };
        let client = Arc::new(McpClient::spawn(&cfg).await.unwrap());

        let mut handles = Vec::new();
        for i in 0..10 {
            let c = client.clone();
            handles.push(tokio::spawn(async move {
                c.call_tool("echo", json!({ "text": format!("msg{i}") }))
                    .await
                    .unwrap()
            }));
        }
        for (i, h) in handles.into_iter().enumerate() {
            let (text, is_error) = h.await.unwrap();
            assert_eq!(text, format!("echo: msg{i}"));
            assert!(!is_error);
        }
    }
}
