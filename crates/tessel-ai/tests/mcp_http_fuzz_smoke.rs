//! Deterministic fuzz-harness tests for untrusted MCP JSON-RPC & tool argument parsing (Phase J4).
//! Verifies zero panics across malformed envelopes, wrong types, boundary numbers, and adversarial strings.

use serde_json::{json, Value};
use tessel_ai::{dispatch_mcp_message, tool_definitions};
use tessel_core::{HistoryStack, SceneState};

#[test]
fn fuzz_mcp_dispatch_and_tool_arguments_never_panic() {
    let export_dir = std::env::temp_dir().join("tessel_j4_mcp_fuzz");
    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();

    // 1. Adversarial JSON-RPC envelopes
    let malformed_envelopes = vec![
        json!(null),
        json!(12345),
        json!("not an object"),
        json!([]),
        json!({}),
        json!({"jsonrpc": "2.0", "id": null, "method": "tools/call", "params": null}),
        json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "scene_batch", "arguments": "not_an_object"}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "../../etc/passwd", "arguments": {}}}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "tessel://unknown"}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 5, "method": "unknown_method", "params": [1, 2, 3]}),
    ];

    for env in &malformed_envelopes {
        let _ = dispatch_mcp_message(env, &mut scene, &mut history, &export_dir);
    }

    // 2. Adversarial argument payloads across all 160 tools
    let adversarial_args: Vec<Value> = vec![
        json!({}),
        json!({"node_id": -1, "size": -999.0, "name": "", "x": 1e308, "y": -1e308, "z": 0}),
        json!({"node_id": "bad_type", "width": 999999999u64, "height": 0, "palette_index": 999}),
        json!({"filename": "../../../etc/passwd", "path": "/etc/shadow", "format": "exe"}),
        json!({"calls": [{"name": "scene_batch", "arguments": {"calls": []}}]}),
    ];

    let defs = tool_definitions();
    for def in &defs {
        let Some(name) = def["name"].as_str() else {
            continue;
        };
        for args in &adversarial_args {
            let req = json!({
                "jsonrpc": "2.0",
                "id": 99,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": args
                }
            });
            let _ = dispatch_mcp_message(&req, &mut scene, &mut history, &export_dir);
        }
    }
}
