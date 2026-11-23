pub mod http;

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use base64::Engine as _;
use serde_json::{json, Value};
use tessel_core::{HistoryStack, SceneState};

use crate::tools::tool_definitions;
use crate::translator::{execute_tool_call, palette_summary_json, scene_summary_json};

pub use http::{LiveMcpRequest, LiveMcpServerHandle};

/// Processes a single JSON-RPC 2.0 MCP request directly against any live `SceneState` & `HistoryStack`.
pub fn dispatch_mcp_message(
    request: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &std::path::Path,
) -> Option<Value> {
    dispatch_mcp_message_with_preview(request, scene, history, allowed_export_dir, None)
}

/// Processes a single JSON-RPC 2.0 MCP request with an optional PNG viewport snapshot for `tessel://viewport/preview`.
pub fn dispatch_mcp_message_with_preview(
    request: &Value,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    allowed_export_dir: &std::path::Path,
    preview_png: Option<&[u8]>,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or("");
    let params = request.get("params").cloned().unwrap_or(Value::Null);

    if method.starts_with("notifications/") {
        return None;
    }

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {},
                "resources": {}
            },
            "serverInfo": {
                "name": "tessel-mcp",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),
        "tools/list" => Ok(json!({
            "tools": tool_definitions()
        })),
        "tools/call" => {
            let tool_name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            match execute_tool_call(tool_name, &args, scene, history, allowed_export_dir) {
                Ok(val) => Ok(json!({
                    "content": [{ "type": "text", "text": val.to_string() }],
                    "isError": false
                })),
                Err(err) => Ok(json!({
                    "content": [{ "type": "text", "text": err.to_structured_json().to_string() }],
                    "isError": true
                })),
            }
        }
        "resources/list" => Ok(json!({
            "resources": [
                {
                    "uri": "tessel://scene/summary",
                    "name": "Scene Summary",
                    "mimeType": "application/json"
                },
                {
                    "uri": "tessel://palette/current",
                    "name": "Active Palette",
                    "mimeType": "application/json"
                },
                {
                    "uri": "tessel://viewport/preview",
                    "name": "Viewport Preview Snapshot",
                    "mimeType": "image/png"
                }
            ]
        })),
        "resources/read" => {
            let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
            match uri {
                "tessel://scene/summary" => Ok(json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": scene_summary_json(scene).to_string()
                    }]
                })),
                "tessel://palette/current" => Ok(json!({
                    "contents": [{
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": palette_summary_json(scene).to_string()
                    }]
                })),
                "tessel://viewport/preview" => match preview_png {
                    Some(bytes) => Ok(json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "image/png",
                            "blob": base64::engine::general_purpose::STANDARD.encode(bytes)
                        }]
                    })),
                    None => Err((
                        -32603,
                        "viewport preview is only available when the Tessel GUI is running".into(),
                    )),
                },
                _ => Err((-32602, format!("unknown resource URI '{uri}'"))),
            }
        }
        other => Err((-32601, format!("method not found: '{other}'"))),
    };

    let response = match result {
        Ok(res) => json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "result": res
        }),
        Err((code, message)) => json!({
            "jsonrpc": "2.0",
            "id": id.unwrap_or(Value::Null),
            "error": { "code": code, "message": message }
        }),
    };

    Some(response)
}

/// Headless JSON-RPC 2.0 MCP Server driving `SceneState` through the single Command Bus.
pub struct McpServer {
    pub scene: SceneState,
    pub history: HistoryStack,
    pub allowed_export_dir: PathBuf,
}

impl McpServer {
    pub fn new(allowed_export_dir: PathBuf) -> Self {
        Self {
            scene: SceneState::new(),
            history: HistoryStack::new(),
            allowed_export_dir,
        }
    }

    /// Processes a single JSON-RPC 2.0 request and returns an optional response (`None` for notifications).
    pub fn handle_message(&mut self, request: &Value) -> Option<Value> {
        dispatch_mcp_message(
            request,
            &mut self.scene,
            &mut self.history,
            &self.allowed_export_dir,
        )
    }

    /// Runs the line-delimited JSON-RPC stdio transport loop until EOF.
    pub fn run_stdio(&mut self) -> io::Result<()> {
        let stdin = io::stdin();
        let mut stdout = io::stdout().lock();
        for line in stdin.lock().lines() {
            let line = line?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<Value>(trimmed) {
                Ok(req) => self.handle_message(&req),
                Err(err) => Some(json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {err}") }
                })),
            };
            if let Some(resp) = response {
                writeln!(stdout, "{resp}")?;
                stdout.flush()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_lifecycle_fill_box_and_export_glb() {
        let export_dir = std::env::temp_dir().join("tessel_mcp_test");
        let _ = std::fs::remove_dir_all(&export_dir);
        let mut server = McpServer::new(export_dir.clone());

        let init = server
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }))
            .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "tessel-mcp");

        let list = server
            .handle_message(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
            .unwrap();
        assert!(!list["result"]["tools"].as_array().unwrap().is_empty());

        let fill = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": {
                    "name": "voxel_fill_box",
                    "arguments": {
                        "min": [0, 0, 0],
                        "max": [2, 2, 2],
                        "palette_index": 4
                    }
                }
            }))
            .unwrap();
        assert_eq!(fill["result"]["isError"], false);

        let exp = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "tools/call",
                "params": {
                    "name": "project_export",
                    "arguments": {
                        "filename": "mcp_cube.glb",
                        "format": "glb",
                        "preset": "godot4"
                    }
                }
            }))
            .unwrap();
        assert_eq!(exp["result"]["isError"], false);
        assert!(export_dir.join("mcp_cube.glb").exists());

        // Path traversal must be rejected cleanly
        let denied = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 5,
                "method": "tools/call",
                "params": {
                    "name": "project_export",
                    "arguments": {
                        "filename": "../escape.glb",
                        "format": "glb"
                    }
                }
            }))
            .unwrap();
        assert_eq!(denied["result"]["isError"], true);
        assert!(denied["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("EXPORT_PATH_DENIED"));

        let _ = std::fs::remove_dir_all(&export_dir);
    }

    #[test]
    fn mcp_rejects_out_of_bounds_and_excessive_batches() {
        let mut server = McpServer::new(std::env::temp_dir());
        let oob = server
            .handle_message(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": {
                    "name": "voxel_fill_box",
                    "arguments": {
                        "min": [0, 0, 0],
                        "max": [300, 1, 1],
                        "palette_index": 4
                    }
                }
            }))
            .unwrap();
        assert_eq!(oob["result"]["isError"], true);
        assert!(oob["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("OUT_OF_BOUNDS"));
    }

    #[test]
    fn mcp_viewport_preview_returns_base64_png_when_available() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let dir = std::env::temp_dir();
        let fake_png = [0x89u8, 0x50, 0x4E, 0x47];
        let resp = dispatch_mcp_message_with_preview(
            &json!({
                "jsonrpc": "2.0",
                "id": 9,
                "method": "resources/read",
                "params": { "uri": "tessel://viewport/preview" }
            }),
            &mut scene,
            &mut history,
            &dir,
            Some(&fake_png),
        )
        .unwrap();
        assert_eq!(resp["result"]["contents"][0]["mimeType"], "image/png");
        assert_eq!(resp["result"]["contents"][0]["blob"], "iVBORw==");
    }
}
