use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde_json::{json, Value};

/// Message sent from the local loopback MCP HTTP server to the live GUI main thread.
pub struct LiveMcpRequest {
    pub payload: Value,
    pub reply_tx: mpsc::Sender<Option<Value>>,
}

/// Handle controlling a running loopback `127.0.0.1` MCP HTTP server.
pub struct LiveMcpServerHandle {
    pub port: u16,
    pub token: String,
    pub rx: mpsc::Receiver<LiveMcpRequest>,
    stop: Arc<AtomicBool>,
}

impl LiveMcpServerHandle {
    pub fn start(port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let bound_port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let token = generate_local_token();
        let expected_auth = format!("Bearer {token}");
        let (tx, rx) = mpsc::channel::<LiveMcpRequest>();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                        handle_http_connection(&mut stream, &expected_auth, &tx);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            port: bound_port,
            token,
            rx,
            stop,
        })
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for LiveMcpServerHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

fn generate_local_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let pid = u128::from(std::process::id());
    format!("tsl_{:016x}{:08x}", nanos ^ (pid << 32), pid)
}

fn handle_http_connection(
    stream: &mut std::net::TcpStream,
    expected_auth: &str,
    tx: &mpsc::Sender<LiveMcpRequest>,
) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);

    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    if !request_line.starts_with("POST /mcp") {
        write_http(stream, "404 Not Found", r#"{"error":"use POST /mcp"}"#);
        return;
    }

    let mut content_length: usize = 0;
    let mut authorized = false;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return;
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            } else if k.eq_ignore_ascii_case("authorization") && v.trim() == expected_auth {
                authorized = true;
            }
        }
    }

    if !authorized {
        write_http(
            stream,
            "401 Unauthorized",
            r#"{"error":"invalid bearer token"}"#,
        );
        return;
    }
    if content_length == 0 || content_length > 2 * 1024 * 1024 {
        write_http(
            stream,
            "400 Bad Request",
            r#"{"error":"invalid content-length"}"#,
        );
        return;
    }

    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        write_http(stream, "400 Bad Request", r#"{"error":"incomplete body"}"#);
        return;
    }

    let payload: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(err) => {
            let msg = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": format!("parse error: {err}") }
            });
            write_http(stream, "400 Bad Request", &msg.to_string());
            return;
        }
    };

    let (reply_tx, reply_rx) = mpsc::channel();
    if tx.send(LiveMcpRequest { payload, reply_tx }).is_err() {
        write_http(
            stream,
            "503 Service Unavailable",
            r#"{"error":"studio unavailable"}"#,
        );
        return;
    }

    match reply_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Some(resp)) => write_http(stream, "200 OK", &resp.to_string()),
        Ok(None) => write_http(stream, "202 Accepted", "{}"),
        Err(_) => write_http(
            stream,
            "504 Gateway Timeout",
            r#"{"error":"studio timeout"}"#,
        ),
    }
}

fn write_http(stream: &mut std::net::TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_mcp_http_enforces_bearer_auth_and_forwards_jsonrpc() {
        let handle = LiveMcpServerHandle::start(0).unwrap();
        let addr = format!("127.0.0.1:{}", handle.port);

        // Unauthorized request must get 401
        let mut s1 = std::net::TcpStream::connect(&addr).unwrap();
        let req1 = "POST /mcp HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}";
        s1.write_all(req1.as_bytes()).unwrap();
        let mut out1 = String::new();
        s1.read_to_string(&mut out1).unwrap();
        assert!(out1.starts_with("HTTP/1.1 401"));

        // Authorized request must reach channel and receive response
        let token = handle.token.clone();
        let client_thread = std::thread::spawn(move || {
            let mut s2 = std::net::TcpStream::connect(&addr).unwrap();
            let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#;
            let req2 = format!(
                "POST /mcp HTTP/1.1\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            s2.write_all(req2.as_bytes()).unwrap();
            let mut out2 = String::new();
            s2.read_to_string(&mut out2).unwrap();
            out2
        });

        let live_req = handle.rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(live_req.payload["method"], "initialize");
        live_req
            .reply_tx
            .send(Some(
                json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }),
            ))
            .unwrap();

        let response_text = client_thread.join().unwrap();
        assert!(response_text.starts_with("HTTP/1.1 200"));
        assert!(response_text.contains("\"ok\":true"));
    }
}
