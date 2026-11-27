use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, RwLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

const SLOW_MCP_DISPATCH_MS: u64 = 100;
const MAX_CONCURRENT_HTTP_WORKERS: usize = 16;

struct ActiveWorkerGuard {
    active: Arc<AtomicUsize>,
}

impl Drop for ActiveWorkerGuard {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

struct McpHttpMetrics {
    requests_total: AtomicU64,
    requests_completed: AtomicU64,
    requests_rejected: AtomicU64,
    requests_timed_out: AtomicU64,
    slow_requests_total: AtomicU64,
    total_latency_us: AtomicU64,
    max_latency_us: AtomicU64,
    last_latency_us: AtomicU64,
    log_limiter: crate::observability::LogRateLimiter,
}

impl Default for McpHttpMetrics {
    fn default() -> Self {
        Self {
            requests_total: AtomicU64::new(0),
            requests_completed: AtomicU64::new(0),
            requests_rejected: AtomicU64::new(0),
            requests_timed_out: AtomicU64::new(0),
            slow_requests_total: AtomicU64::new(0),
            total_latency_us: AtomicU64::new(0),
            max_latency_us: AtomicU64::new(0),
            last_latency_us: AtomicU64::new(0),
            log_limiter: crate::observability::LogRateLimiter::new(Duration::from_secs(10), 3),
        }
    }
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
        let trimmed = token.trim().to_string();
        if trimmed.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "MCP bearer token must not be empty",
            ));
        }
        let listener = TcpListener::bind(("127.0.0.1", port))?;
        let bound_port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;

        let expected_auth = Arc::new(RwLock::new(format!("Bearer {trimmed}")));
        let auth_ref = Arc::clone(&expected_auth);
        let (tx, rx) = mpsc::sync_channel::<LiveMcpRequest>(MAX_MCP_QUEUE);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let metrics = Arc::new(McpHttpMetrics::default());
        let metrics_ref = Arc::clone(&metrics);
        let active_workers = Arc::new(AtomicUsize::new(0));

        let join_handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let prev = active_workers.fetch_add(1, Ordering::AcqRel);
                        if prev >= MAX_CONCURRENT_HTTP_WORKERS {
                            active_workers.fetch_sub(1, Ordering::AcqRel);
                            metrics_ref
                                .requests_rejected
                                .fetch_add(1, Ordering::Relaxed);
                            if let crate::observability::LogDecision::Emit { suppressed } =
                                metrics_ref.log_limiter.check("worker_pool_full")
                            {
                                tracing::warn!(
                                    suppressed,
                                    "Rejected MCP HTTP connection: worker pool full"
                                );
                            }
                            let _ = stream.set_nonblocking(false);
                            let _ = stream.set_write_timeout(Some(Duration::from_millis(100)));
                            write_http(
                                &mut stream,
                                "503 Service Unavailable",
                                r#"{"error":"too many concurrent MCP connections"}"#,
                            );
                            continue;
                        }
                        let guard = ActiveWorkerGuard {
                            active: Arc::clone(&active_workers),
                        };
                        let current_auth = auth_ref.read().map(|g| g.clone()).unwrap_or_default();
                        let tx_clone = tx.clone();
                        let metrics_clone = Arc::clone(&metrics_ref);
                        let stop_clone = Arc::clone(&stop_flag);
                        let spawn_res = std::thread::Builder::new()
                            .name("tessel-mcp-http-worker".into())
                            .spawn(move || {
                                let _guard = guard;
                                let _ = stream.set_nonblocking(false);
                                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                                let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                                handle_http_connection(
                                    &mut stream,
                                    &current_auth,
                                    &tx_clone,
                                    &metrics_clone,
                                    &stop_clone,
                                );
                            });
                        if let Err(err) = spawn_res {
                            if let crate::observability::LogDecision::Emit { suppressed } =
                                metrics_ref.log_limiter.check("worker_spawn_failed")
                            {
                                tracing::warn!(
                                    suppressed,
                                    error = %err,
                                    "Failed to spawn MCP HTTP worker thread"
                                );
                            }
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(15));
                    }
                    Err(err) => {
                        tracing::warn!(error = %err, "MCP HTTP listener accept error; stopping");
                        break;
                    }
                }
            }
        });

        Ok(Self {
            port: bound_port,
            token: trimmed,
            rx,
            expected_auth,
            stop,
            join_handle: Some(join_handle),
        })
    }

    /// Rotates the active bearer token in place without restarting the listening socket.
    /// Ignores empty/whitespace-only tokens to prevent disabling authentication.
    pub fn set_token(&mut self, new_token: String) {
        let trimmed = new_token.trim().to_string();
        if trimmed.is_empty() {
            tracing::warn!("Refused to rotate MCP bearer token to an empty value");
            return;
        }
        let mut guard = self
            .expected_auth
            .write()
            .unwrap_or_else(|e| e.into_inner());
        *guard = format!("Bearer {trimmed}");
        self.token = trimmed;
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
    let mut bytes = [0u8; 16];
    if getrandom::fill(&mut bytes).is_ok() {
        return format_token_hex(&bytes);
    }
    #[cfg(unix)]
    {
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            if f.read_exact(&mut bytes).is_ok() {
                return format_token_hex(&bytes);
            }
        }
    }
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let pid = u64::from(std::process::id());
    let mut h1 = RandomState::new().build_hasher();
    h1.write_u128(nanos);
    h1.write_u64(pid);
    let mut h2 = RandomState::new().build_hasher();
    h2.write_u64(h1.finish());
    h2.write_u128(nanos.rotate_left(17));
    bytes[..8].copy_from_slice(&h1.finish().to_le_bytes());
    bytes[8..].copy_from_slice(&h2.finish().to_le_bytes());
    format_token_hex(&bytes)
}

fn format_token_hex(bytes: &[u8; 16]) -> String {
    let mut hex = String::with_capacity(36);
    hex.push_str("tsl_");
    for b in bytes {
        let _ = std::fmt::Write::write_fmt(&mut hex, format_args!("{b:02x}"));
    }
    hex
}

const MAX_HTTP_HEADER_LINE_BYTES: u64 = 8 * 1024;
const MAX_HTTP_HEADERS: usize = 64;
const MAX_HTTP_READ_DEADLINE: Duration = Duration::from_secs(5);

enum LineReadError {
    Io,
    TooLong,
}

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
    let trimmed = host.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(rest) = trimmed.strip_prefix('[') {
        let Some((addr, after)) = rest.split_once(']') else {
            return false;
        };
        if !addr.eq_ignore_ascii_case("::1") {
            return false;
        }
        return after.is_empty()
            || after
                .strip_prefix(':')
                .is_some_and(|p| p.parse::<u16>().is_ok_and(|n| n > 0));
    }
    let (name, port) = match trimmed.split_once(':') {
        Some((n, p)) => (n, Some(p)),
        None => (trimmed, None),
    };
    if let Some(p) = port {
        if !p.parse::<u16>().is_ok_and(|n| n > 0) {
            return false;
        }
    }
    matches!(
        name.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost"
    )
}

fn is_loopback_origin(origin: &str) -> bool {
    let trimmed = origin.trim();
    let Ok(parsed) = url::Url::parse(trimmed) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    if !matches!(parsed.path(), "" | "/") || parsed.query().is_some() || parsed.fragment().is_some()
    {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
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

fn read_bounded_line<R: BufRead>(reader: &mut R, out: &mut String) -> Result<usize, LineReadError> {
    out.clear();
    let n = reader
        .by_ref()
        .take(MAX_HTTP_HEADER_LINE_BYTES + 1)
        .read_line(out)
        .map_err(|_| LineReadError::Io)?;
    if n as u64 > MAX_HTTP_HEADER_LINE_BYTES
        || (n > 0 && !out.ends_with('\n') && n as u64 >= MAX_HTTP_HEADER_LINE_BYTES)
    {
        return Err(LineReadError::TooLong);
    }
    Ok(n)
}

fn handle_http_connection(
    stream: &mut std::net::TcpStream,
    expected_auth: &str,
    tx: &mpsc::SyncSender<LiveMcpRequest>,
    metrics: &McpHttpMetrics,
    stop: &AtomicBool,
) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);

    let read_start = Instant::now();
    let mut request_line = String::new();
    match read_bounded_line(&mut reader, &mut request_line) {
        Ok(0) | Err(LineReadError::Io) => return,
        Err(LineReadError::TooLong) => {
            metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
            write_http(
                stream,
                "431 Request Header Fields Too Large",
                r#"{"error":"request line too long"}"#,
            );
            return;
        }
        Ok(_) => {}
    }

    let is_health = request_line.starts_with("GET /health ");
    let is_mcp = request_line.starts_with("POST /mcp ");
    if !is_health && !is_mcp {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        write_http(
            stream,
            "404 Not Found",
            r#"{"error":"use POST /mcp or GET /health"}"#,
        );
        return;
    }
    if is_mcp {
        metrics.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    let mut content_length: Option<usize> = None;
    let mut bad_framing = false;
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
        match read_bounded_line(&mut reader, &mut header) {
            Ok(0) => break,
            Ok(_) => {}
            Err(LineReadError::Io) => return,
            Err(LineReadError::TooLong) => {
                metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
                write_http(
                    stream,
                    "431 Request Header Fields Too Large",
                    r#"{"error":"header line too long"}"#,
                );
                return;
            }
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
        let Some((k, v)) = trimmed.split_once(':') else {
            bad_framing = true;
            continue;
        };
        if k.is_empty() || k.bytes().any(|b| b.is_ascii_whitespace()) {
            bad_framing = true;
            continue;
        }
        if k.eq_ignore_ascii_case("content-length") {
            if content_length.is_some() {
                bad_framing = true;
            } else {
                match v.trim().parse::<usize>() {
                    Ok(n) => content_length = Some(n),
                    Err(_) => bad_framing = true,
                }
            }
        } else if k.eq_ignore_ascii_case("transfer-encoding") {
            bad_framing = true;
        } else if k.eq_ignore_ascii_case("authorization")
            && !expected_auth.is_empty()
            && constant_time_eq(v.trim().as_bytes(), expected_auth.as_bytes())
        {
            authorized = true;
        } else if (k.eq_ignore_ascii_case("host") && !is_loopback_host(v))
            || (k.eq_ignore_ascii_case("origin") && !is_loopback_origin(v))
        {
            bad_host_or_origin = true;
        }
    }

    if bad_host_or_origin {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        if let crate::observability::LogDecision::Emit { suppressed } =
            metrics.log_limiter.check("bad_host_or_origin")
        {
            tracing::warn!(
                suppressed,
                "Rejected MCP HTTP request with non-loopback host or origin"
            );
        }
        write_http(
            stream,
            "403 Forbidden",
            r#"{"error":"loopback host/origin required"}"#,
        );
        return;
    }

    if !authorized {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        if let crate::observability::LogDecision::Emit { suppressed } =
            metrics.log_limiter.check("unauthorized")
        {
            tracing::warn!(suppressed, "Rejected unauthorized MCP HTTP request");
        }
        write_http(
            stream,
            "401 Unauthorized",
            r#"{"error":"invalid bearer token"}"#,
        );
        return;
    }

    if bad_framing {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        write_http(
            stream,
            "400 Bad Request",
            r#"{"error":"invalid or unsupported framing headers"}"#,
        );
        return;
    }

    if is_health {
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

    let content_length = content_length.unwrap_or(0);
    if content_length == 0 || content_length > 2 * 1024 * 1024 {
        metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
        write_http(
            stream,
            "400 Bad Request",
            r#"{"error":"invalid content-length"}"#,
        );
        return;
    }

    let mut body = Vec::with_capacity(content_length.min(64 * 1024));
    let mut chunk_buf = [0u8; 4096];
    while body.len() < content_length {
        if read_start.elapsed() > MAX_HTTP_READ_DEADLINE {
            metrics.requests_timed_out.fetch_add(1, Ordering::Relaxed);
            write_http(
                stream,
                "408 Request Timeout",
                r#"{"error":"body read timeout"}"#,
            );
            return;
        }
        let want = (content_length - body.len()).min(chunk_buf.len());
        match reader.read(&mut chunk_buf[..want]) {
            Ok(0) => {
                write_http(stream, "400 Bad Request", r#"{"error":"incomplete body"}"#);
                return;
            }
            Ok(n) => body.extend_from_slice(&chunk_buf[..n]),
            Err(_) => {
                write_http(stream, "400 Bad Request", r#"{"error":"incomplete body"}"#);
                return;
            }
        }
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

    let deadline = Instant::now() + timeout;
    loop {
        if stop.load(Ordering::Relaxed) {
            write_http(
                stream,
                "503 Service Unavailable",
                r#"{"error":"studio shutting down"}"#,
            );
            return;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
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
            );
            return;
        }
        match reply_rx.recv_timeout(remaining.min(Duration::from_millis(50))) {
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
                write_http(stream, "200 OK", &resp.to_string());
                return;
            }
            Ok(None) => {
                metrics.record_latency(started.elapsed());
                write_http(stream, "202 Accepted", "{}");
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                metrics.requests_rejected.fetch_add(1, Ordering::Relaxed);
                write_http(
                    stream,
                    "503 Service Unavailable",
                    r#"{"error":"studio unavailable"}"#,
                );
                return;
            }
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

        // Unauthorized GET /health must get 401
        let mut s3_unauth = std::net::TcpStream::connect(&addr).unwrap();
        s3_unauth
            .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
            .unwrap();
        let mut out3_unauth = String::new();
        s3_unauth.read_to_string(&mut out3_unauth).unwrap();
        assert!(out3_unauth.starts_with("HTTP/1.1 401"));

        // Authorized GET /health returns 200 OK with counters
        let mut s3 = std::net::TcpStream::connect(&addr).unwrap();
        let health_req = format!(
            "GET /health HTTP/1.1\r\nHost: [::1]:{}\r\nAuthorization: Bearer {}\r\n\r\n",
            handle.port, handle.token
        );
        s3.write_all(health_req.as_bytes()).unwrap();
        let mut out3 = String::new();
        s3.read_to_string(&mut out3).unwrap();
        assert!(out3.starts_with("HTTP/1.1 200"));
        assert!(out3.contains("\"status\":\"ok\""));
        assert!(out3.contains("\"requests_total\":2"));
        assert!(out3.contains("\"requests_completed\":1"));
        assert!(out3.contains("\"requests_rejected\":2"));
        assert!(out3.contains("\"avg_latency_ms\":"));
        assert!(out3.contains("\"max_latency_ms\":"));
        assert!(out3.contains("\"slow_requests_total\":"));
    }

    #[test]
    fn hardening_rejects_spoofed_origin_bad_framing_oversized_header_and_empty_token() {
        assert!(LiveMcpServerHandle::start_with_token(0, "   ".into()).is_err());
        assert!(is_loopback_host("127.0.0.1:38421"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("[::1]"));
        assert!(is_loopback_host("[::1]:38421"));
        assert!(!is_loopback_host("localhost.attacker.example"));
        assert!(!is_loopback_host("[::1]:0"));

        assert!(is_loopback_origin("http://127.0.0.1:38421"));
        assert!(is_loopback_origin("http://localhost:8080"));
        assert!(is_loopback_origin("http://[::1]:38421"));
        assert!(!is_loopback_origin("http://localhost.attacker.example"));
        assert!(!is_loopback_origin("http://127.0.0.1.evil.com"));
        assert!(!is_loopback_origin("http://user:pass@localhost"));

        let handle = LiveMcpServerHandle::start(0).unwrap();
        let addr = format!("127.0.0.1:{}", handle.port);

        // Spoofed Origin -> 403
        let mut s = std::net::TcpStream::connect(&addr).unwrap();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://localhost.attacker.example\r\nAuthorization: Bearer {}\r\nContent-Length: 2\r\n\r\n{{}}",
            handle.token
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 403"), "got: {out}");

        // Duplicate Content-Length -> 400
        let mut s = std::net::TcpStream::connect(&addr).unwrap();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nContent-Length: 2\r\nContent-Length: 2\r\n\r\n{{}}",
            handle.token
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 400"), "got: {out}");

        // Transfer-Encoding -> 400
        let mut s = std::net::TcpStream::connect(&addr).unwrap();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
            handle.token
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 400"), "got: {out}");

        // Oversized header > 8 KiB -> 431
        let mut s = std::net::TcpStream::connect(&addr).unwrap();
        let big = "a".repeat(9000);
        let req = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Big: {big}\r\nAuthorization: Bearer {}\r\nContent-Length: 2\r\n\r\n{{}}",
            handle.token
        );
        s.write_all(req.as_bytes()).unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 431"), "got: {out}");
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
