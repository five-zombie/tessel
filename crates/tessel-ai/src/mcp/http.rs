use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const SLOW_MCP_DISPATCH_MS: u64 = 100;

#[derive(Default)]
struct McpHttpMetrics {
    requests_total: AtomicU64,
    requests_completed: AtomicU64,
    requests_rejected: AtomicU64,
    requests_timed_out: AtomicU64,
    slow_requests_total: AtomicU64,
    total_latency_us: AtomicU64,
    max_latency_us: AtomicU64,
    last_latency_us: AtomicU64,
}

impl McpHttpMetrics {
    fn record_latency(&self, elapsed: Duration) {
        let us = elapsed.as_micros().min(u128::from(u64::MAX)) as u64;
        self.requests_completed.fetch_add(1, Ordering::Relaxed);
        self.total_latency_us.fetch_add(us, Ordering::Relaxed);
        self.last_latency_us.store(us, Ordering::Relaxed);
        self.max_latency_us.fetch_max(us, Ordering::Relaxed);
        if elapsed.as_millis() as u64 >= SLOW_MCP_DISPATCH_MS {
            self.slow_requests_total.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Message sent from the local loopback MCP HTTP server to the live GUI main thread.
pub struct LiveMcpRequest {
    pub payload: Value,
    pub reply_tx: mpsc::Sender<Option<Value>>,
}

const MAX_MCP_QUEUE: usize = 64;

/// Handle controlling a running loopback `127.0.0.1` MCP HTTP server.
pub struct LiveMcpServerHandle {
    pub port: u16,
    pub token: String,
    pub rx: mpsc::Receiver<LiveMcpRequest>,
    expected_auth: Arc<RwLock<String>>,
    stop: Arc<AtomicBool>,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl LiveMcpServerHandle {
    pub fn start(port: u16) -> std::io::Result<Self> {
        Self::start_with_token(port, generate_local_token())
    }

    pub fn start_with_token(port: u16, token: String) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let bound_port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let expected_auth = Arc::new(RwLock::new(format!("Bearer {token}")));
        let auth_ref = Arc::clone(&expected_auth);
        let (tx, rx) = mpsc::sync_channel::<LiveMcpRequest>(MAX_MCP_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let metrics = Arc::new(McpHttpMetrics::default());
        let metrics_ref = Arc::clone(&metrics);

        let join_handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        let current_auth = auth_ref.read().map(|g| g.clone()).unwrap_or_default();
                        handle_http_connection(&mut stream, &current_auth, &tx, &metrics_ref);
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
            expected_auth,
            stop,
            join_handle: Some(join_handle),
        })
    }

    /// Rotates the active bearer token in place without restarting the listening socket.
    pub fn set_token(&mut self, new_token: String) {
        let mut guard = self
            .expected_auth
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = format!("Bearer {new_token}");
        self.token = new_token;
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Drop for LiveMcpServerHandle {
    fn drop(&mut self) {
        self.stop();
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn generate_local_token() -> String {
    #[cfg(unix)]
    {
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let mut bytes = [0u8; 16];
            if f.read_exact(&mut bytes).is_ok() {
                let mut hex = String::with_capacity(36);
                hex.push_str("tsl_");
                for b in bytes {
                    let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{b:02x}"));
                }
                return hex;
            }
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let pid = u128::from(std::process::id());
    format!("tsl_{:016x}{:08x}", nanos ^ (pid << 32), pid)
}

const MAX_HTTP_HEADER_LINE_BYTES: u64 = 8 * 1024;
const MAX_HTTP_HEADERS: usize = 64;
const MAX_HTTP_READ_DEADLINE: Duration = Duration::from_secs(5);

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn is_loopback_host(host: &str) -> bool {
    let bare = host
        .trim()
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(bare.as_str(), "127.0.0.1" | "localhost" | "[::1]")
}

fn request_timeout_for_payload(payload: &Value) -> Duration {
    let tool_name = payload
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match tool_name {
        "viewport_render_turntable_gif"
        | "viewport_screenshot"
        | "project_export"
        | "scene_batch"
        | "mesh_bake_maps"
        | "mesh_fill_smart_material" => Duration::from_secs(120),
        _ => Duration::from_secs(15),
    }
}

fn read_bounded_line<R: BufRead>(reader: &mut R, out: &mut String) -> std::io::Result<usize> {
    out.clear();
    reader
        .by_ref()
        .take(MAX_HTTP_HEADER_LINE_BYTES)
        .read_line(out)
}

fn handle_http_connection(
    stream: &mut std::net::TcpStream,
    expected_auth: &str,
    tx: &mpsc::SyncSender<LiveMcpRequest>,
    metrics: &McpHttpMetrics,
) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);

    let read_start = Instant::now();
    let mut request_line = String::new();
    if read_bounded_line(&mut reader, &mut request_line).is_err() {
        return;
    }
    if request_line.starts_with("GET /health ") {
        let completed = metrics.requests_completed.load(Ordering::Relaxed);
        let total_us = metrics.total_latency_us.load(Ordering::Relaxed);
        let avg_latency_ms = if completed > 0 {
            (total_us as f64 / completed as f64) / 1000.0
        } else {
            0.0
        };
        let max_latency_ms = metrics.max_latency_us.load(Ordering::Relaxed) as f64 / 1000.0;
        let last_latency_ms = metrics.last_latency_us.load(Ordering::Relaxed) as f64 / 1000.0;
        let health = json!({
            "status": "ok",
            "queue_capacity": MAX_MCP_QUEUE,
            "requests_total": metrics.requests_total.load(Ordering::Relaxed),
            "requests_completed": completed,
            "requests_rejected": metrics.requests_rejected.load(Ordering::Relaxed),
            "requests_timed_out": metrics.requests_timed_out.load(Ordering::Relaxed),
            "slow_requests_total": metrics.slow_requests_total.load(Ordering::Relaxed),
            "avg_latency_ms": avg_latency_ms,
            "max_latency_ms": max_latency_ms,
            "last_latency_ms": last_latency_ms,
        });
        write_http(stream, "200 OK", &health.to_string());
        return;
    }
    if !request_line.starts_with("POST /mcp ") {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        write_http(
            stream,
            "404 Not Found",
            r#"{"error":"use POST /mcp or GET /health"}"#,
        );
        return;
    }
    metrics.requests_total.fetch_add(1, Ordering::Relaxed);

    let mut content_length: usize = 0;
    let mut authorized = false;
    let mut bad_host_or_origin = false;
    let mut header_count: usize = 0;
    let mut header = String::new();
    loop {
        if read_start.elapsed() > MAX_HTTP_READ_DEADLINE {
            metrics.requests_timed_out.fetch_add(1, Ordering::Relaxed);
            write_http(
                stream,
                "408 Request Timeout",
                r#"{"error":"header timeout"}"#,
            );
            return;
        }
        if read_bounded_line(&mut reader, &mut header).is_err() {
            return;
        }
        let trimmed = header.trim();
        if trimmed.is_empty() {
            break;
        }
        header_count += 1;
        if header_count > MAX_HTTP_HEADERS {
            metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
            write_http(
                stream,
                "431 Request Header Fields Too Large",
                r#"{"error":"too many headers"}"#,
            );
            return;
        }
        if let Some((k, v)) = trimmed.split_once(':') {
            if k.eq_ignore_ascii_case("content-length") {
                content_length = v.trim().parse().unwrap_or(0);
            } else if k.eq_ignore_ascii_case("authorization")
                && constant_time_eq(v.trim().as_bytes(), expected_auth.as_bytes())
            {
                authorized = true;
            } else if k.eq_ignore_ascii_case("host") && !is_loopback_host(v) {
                bad_host_or_origin = true;
            } else if k.eq_ignore_ascii_case("origin") {
                let origin = v.trim();
                let ok_origin = origin.starts_with("http://127.0.0.1")
                    || origin.starts_with("http://localhost");
                if !ok_origin {
                    bad_host_or_origin = true;
                }
            }
        }
    }

    if bad_host_or_origin {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        write_http(
            stream,
            "403 Forbidden",
            r#"{"error":"loopback host/origin required"}"#,
        );
        return;
    }

    if !authorized {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        tracing::warn!("Rejected unauthorized MCP HTTP request");
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

    let timeout = request_timeout_for_payload(&payload);
    let tool_label = payload
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("method").and_then(Value::as_str))
        .unwrap_or("unknown")
        .to_string();
    let started = Instant::now();
    let (reply_tx, reply_rx) = mpsc::channel();
    match tx.try_send(LiveMcpRequest { payload, reply_tx }) {
        Ok(()) => {}
        Err(mpsc::TrySendError::Full(_)) => {
            metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                queue_capacity = MAX_MCP_QUEUE,
                "MCP HTTP queue full; shedding request"
            );
            write_http(
                stream,
                "429 Too Many Requests",
                r#"{"error":"studio busy; request queue full"}"#,
            );
            return;
        }
        Err(mpsc::TrySendError::Disconnected(_)) => {
            metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
            write_http(
                stream,
                "503 Service Unavailable",
                r#"{"error":"studio unavailable"}"#,
            );
            return;
        }
    }

    match reply_rx.recv_timeout(timeout) {
        Ok(Some(resp)) => {
            let elapsed = started.elapsed();
            metrics.record_latency(elapsed);
            let elapsed_ms = elapsed.as_millis() as u64;
            if elapsed_ms >= SLOW_MCP_DISPATCH_MS {
                tracing::warn!(
                    tool = %tool_label,
                    elapsed_ms,
                    "Slow live MCP HTTP dispatch on studio main thread"
                );
            }
            write_http(stream, "200 OK", &resp.to_string())
        }
        Ok(None) => {
            metrics.record_latency(started.elapsed());
            write_http(stream, "202 Accepted", "{}")
        }
        Err(_) => {
            metrics.requests_timed_out.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                tool = %tool_label,
                timeout_s = timeout.as_secs(),
                "MCP HTTP request timed out waiting for studio main thread"
            );
            write_http(
                stream,
                "504 Gateway Timeout",
                r#"{"error":"studio timeout"}"#,
            )
        }
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
        let addr_c = addr.clone();
        let client_thread = std::thread::spawn(move || {
            let mut s2 = std::net::TcpStream::connect(&addr_c).unwrap();
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

        // GET /health returns 200 OK with counters reflecting both requests
        let mut s3 = std::net::TcpStream::connect(&addr).unwrap();
        s3.write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut out3 = String::new();
        s3.read_to_string(&mut out3).unwrap();
        assert!(out3.starts_with("HTTP/1.1 200"));
        assert!(out3.contains("\"status\":\"ok\""));
        assert!(out3.contains("\"requests_total\":2"));
        assert!(out3.contains("\"requests_completed\":1"));
        assert!(out3.contains("\"requests_rejected\":1"));
        assert!(out3.contains("\"avg_latency_ms\":"));
        assert!(out3.contains("\"max_latency_ms\":"));
        assert!(out3.contains("\"slow_requests_total\":"));
    }

    #[test]
    fn heavy_render_tools_get_extended_http_timeout() {
        let gif_req = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "viewport_render_turntable_gif", "arguments": {} }
        });
        assert_eq!(
            request_timeout_for_payload(&gif_req),
            Duration::from_secs(120)
        );

        let inspect_req = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": { "name": "scene_inspect", "arguments": {} }
        });
        assert_eq!(
            request_timeout_for_payload(&inspect_req),
            Duration::from_secs(15)
        );
    }
}
