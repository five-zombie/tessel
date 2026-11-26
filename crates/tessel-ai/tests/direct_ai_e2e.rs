use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

use serde_json::{json, Value};
use tessel_ai::direct::{send_prompt, AiProviderKind, DirectAiConfig};
use tessel_ai::translator::execute_tool_call;
use tessel_ai::AiError;
use tessel_core::{HistoryStack, NodeId, SceneState};

struct CapturedRequest {
    request_line: String,
    headers: String,
    body: String,
}

fn spawn_single_response_server(
    status_line: &'static str,
    response_body: String,
) -> (String, mpsc::Receiver<CapturedRequest>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback test listener");
    let addr = listener.local_addr().expect("local addr");
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let mut header_end = None;
        let mut content_len = 0usize;

        while let Ok(n) = stream.read(&mut chunk) {
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if header_end.is_none() {
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = Some(pos + 4);
                    let headers_str = String::from_utf8_lossy(&buf[..pos]);
                    for line in headers_str.lines() {
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                content_len = v.trim().parse().unwrap_or(0);
                            }
                        }
                    }
                }
            }
            if let Some(end) = header_end {
                if buf.len() >= end + content_len {
                    break;
                }
            }
        }

        let end = header_end.unwrap_or(buf.len());
        let head = String::from_utf8_lossy(&buf[..end]).into_owned();
        let request_line = head.lines().next().unwrap_or("").to_string();
        let body = String::from_utf8_lossy(&buf[end..]).into_owned();
        let _ = tx.send(CapturedRequest {
            request_line,
            headers: head,
            body,
        });

        let http_resp = format!(
            "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let _ = stream.write_all(http_resp.as_bytes());
        let _ = stream.flush();
    });

    (format!("http://127.0.0.1:{}", addr.port()), rx)
}

#[test]
fn direct_ai_e2e_openai_tool_calls_execute_and_undo_on_scene() {
    let response_json = json!({
        "choices": [{
            "message": {
                "content": "Created a centered cube and moved it to [1, 2, 0].",
                "tool_calls": [
                    {
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "mesh_create_primitive",
                            "arguments": "{\"name\":\"E2ECube\",\"kind\":\"cube\",\"size\":2.0,\"anchor\":\"center\"}"
                        }
                    },
                    {
                        "id": "call_2",
                        "type": "function",
                        "function": {
                            "name": "scene_set_node_transform",
                            "arguments": "{\"position\":[1.0,2.0,0.0]}"
                        }
                    }
                ]
            }
        }]
    })
    .to_string();

    let (base_url, rx) = spawn_single_response_server("200 OK", response_json);
    let config = DirectAiConfig {
        provider: AiProviderKind::OpenAi,
        model: "gpt-4o-mini".into(),
        base_url: format!("{base_url}/v1/chat/completions"),
        api_key: "sk-e2e-test-key".into(),
    };

    let dir = std::env::temp_dir().join("tessel_c2_direct_ai_e2e");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut scene = SceneState::new();
    let mut history = HistoryStack::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let resp = rt
        .block_on(send_prompt(&config, &scene, "create a centered cube"))
        .expect("OpenAI loopback E2E call must succeed");

    let req = rx.recv().expect("server must receive request");
    assert!(req.request_line.starts_with("POST /v1/chat/completions"));
    assert!(req
        .headers
        .to_ascii_lowercase()
        .contains("authorization: bearer sk-e2e-test-key"));
    let sent_body: Value = serde_json::from_str(&req.body).expect("valid JSON request body");
    assert_eq!(sent_body["model"], "gpt-4o-mini");
    assert_eq!(sent_body["tools"].as_array().unwrap().len(), 146);

    assert_eq!(resp.tool_calls.len(), 2);
    for call in &resp.tool_calls {
        execute_tool_call(&call.name, &call.arguments, &mut scene, &mut history, &dir)
            .expect("tool call from Direct AI must succeed");
    }

    let nid: NodeId = scene.selected_node.expect("cube must be selected");
    assert_eq!(
        scene.nodes[&nid].transform.translation,
        glam::Vec3::new(1.0, 2.0, 0.0)
    );
    let (lo, hi) = scene.lowpoly_meshes[&nid].bounds();
    assert!((lo.y + 1.0).abs() < 1e-4 && (hi.y - 1.0).abs() < 1e-4);

    // Undo both tool calls cleanly via unified Command Bus.
    history.undo(&mut scene).unwrap();
    assert_eq!(scene.nodes[&nid].transform.translation, glam::Vec3::ZERO);
    history.undo(&mut scene).unwrap();
    assert!(scene.lowpoly_meshes.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn direct_ai_e2e_ollama_local_and_error_redaction() {
    let ok_json = json!({
        "choices": [{
            "message": { "content": "Local Ollama ready." }
        }]
    })
    .to_string();
    let (ok_url, ok_rx) = spawn_single_response_server("200 OK", ok_json);
    let ollama_cfg = DirectAiConfig {
        provider: AiProviderKind::Ollama,
        model: "llama3.2".into(),
        base_url: format!("{ok_url}/v1/chat/completions"),
        api_key: String::new(),
    };
    let scene = SceneState::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let resp = rt
        .block_on(send_prompt(&ollama_cfg, &scene, "hello"))
        .expect("Ollama without API key must succeed");
    assert_eq!(resp.reply, "Local Ollama ready.");
    let req = ok_rx.recv().unwrap();
    assert!(!req.headers.to_ascii_lowercase().contains("authorization:"));

    // Provider 401 error echoing secrets must be redacted before surfacing.
    let err_json = json!({
        "error": {
            "message": "Invalid key sk-live-secret999 and Bearer tok_abc123"
        }
    })
    .to_string();
    let (err_url, _err_rx) = spawn_single_response_server("401 Unauthorized", err_json);
    let bad_cfg = DirectAiConfig {
        provider: AiProviderKind::OpenAi,
        model: "gpt-4o-mini".into(),
        base_url: format!("{err_url}/v1/chat/completions"),
        api_key: "sk-live-secret999".into(),
    };
    let err = rt
        .block_on(send_prompt(&bad_cfg, &scene, "hello"))
        .unwrap_err();
    match err {
        AiError::ProviderStatus { status, message } => {
            assert_eq!(status, 401);
            assert!(message.contains("<redacted>"));
            assert!(!message.contains("sk-live-secret999"));
            assert!(!message.contains("tok_abc123"));
        }
        other => panic!("expected ProviderStatus, got {other:?}"),
    }
}

#[test]
fn direct_ai_e2e_gemini_and_anthropic_auth_headers() {
    let gem_json = json!({
        "candidates": [{
            "content": { "parts": [{ "text": "Gemini reply OK" }] }
        }]
    })
    .to_string();
    let (gem_url, gem_rx) = spawn_single_response_server("200 OK", gem_json);
    let gem_cfg = DirectAiConfig {
        provider: AiProviderKind::Gemini,
        model: "gemini-2.5-flash".into(),
        base_url: gem_url,
        api_key: "AIzaTestHeaderKey".into(),
    };
    let scene = SceneState::new();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let gem_resp = rt
        .block_on(send_prompt(&gem_cfg, &scene, "ping"))
        .expect("Gemini loopback E2E must succeed");
    assert_eq!(gem_resp.reply, "Gemini reply OK");
    let gem_req = gem_rx.recv().unwrap();
    assert!(!gem_req.request_line.contains("AIzaTestHeaderKey"));
    assert!(gem_req
        .headers
        .to_ascii_lowercase()
        .contains("x-goog-api-key: aizatestheaderkey"));

    let ant_json = json!({
        "content": [{ "text": "Anthropic reply OK" }]
    })
    .to_string();
    let (ant_url, ant_rx) = spawn_single_response_server("200 OK", ant_json);
    let ant_cfg = DirectAiConfig {
        provider: AiProviderKind::Anthropic,
        model: "claude-3-5-sonnet-latest".into(),
        base_url: format!("{ant_url}/v1/messages"),
        api_key: "sk-ant-test".into(),
    };
    let ant_resp = rt
        .block_on(send_prompt(&ant_cfg, &scene, "ping"))
        .expect("Anthropic loopback E2E must succeed");
    assert_eq!(ant_resp.reply, "Anthropic reply OK");
    let ant_req = ant_rx.recv().unwrap();
    assert!(ant_req
        .headers
        .to_ascii_lowercase()
        .contains("x-api-key: sk-ant-test"));
    assert!(ant_req
        .headers
        .to_ascii_lowercase()
        .contains("anthropic-version: 2023-06-01"));
}
