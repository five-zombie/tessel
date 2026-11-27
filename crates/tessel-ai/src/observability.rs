use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_RATE_LIMIT_KEYS: usize = 64;

/// Decision returned by [`LogRateLimiter::check`] to prevent log storms in hot paths or loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogDecision {
    /// Emit the log event, optionally noting how many duplicate events were suppressed since the last emit.
    Emit { suppressed: u64 },
    /// Suppress this log event because the key exceeded its burst budget within the active window.
    Suppress,
}

#[derive(Debug)]
struct BucketState {
    window_start: Instant,
    emitted_in_window: u32,
    suppressed: u64,
}

/// Bounded-cardinality, thread-safe log rate limiter to prevent log storms.
#[derive(Debug)]
pub struct LogRateLimiter {
    window: Duration,
    max_per_window: u32,
    buckets: Mutex<HashMap<&'static str, BucketState>>,
}

impl LogRateLimiter {
    pub fn new(window: Duration, max_per_window: u32) -> Self {
        Self {
            window,
            max_per_window: max_per_window.max(1),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Evaluates whether a log event for `key` should be emitted at `now`.
    pub fn check_at(&self, key: &'static str, now: Instant) -> LogDecision {
        let mut guard = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if !guard.contains_key(key) && guard.len() >= MAX_RATE_LIMIT_KEYS {
            if let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, b)| b.window_start)
                .map(|(k, _)| *k)
            {
                guard.remove(oldest_key);
            }
        }
        let bucket = guard.entry(key).or_insert_with(|| BucketState {
            window_start: now,
            emitted_in_window: 0,
            suppressed: 0,
        });
        if now.duration_since(bucket.window_start) >= self.window {
            let suppressed = bucket.suppressed;
            bucket.window_start = now;
            bucket.emitted_in_window = 1;
            bucket.suppressed = 0;
            return LogDecision::Emit { suppressed };
        }
        if bucket.emitted_in_window < self.max_per_window {
            bucket.emitted_in_window += 1;
            let suppressed = bucket.suppressed;
            bucket.suppressed = 0;
            LogDecision::Emit { suppressed }
        } else {
            bucket.suppressed = bucket.suppressed.saturating_add(1);
            LogDecision::Suppress
        }
    }

    pub fn check(&self, key: &'static str) -> LogDecision {
        self.check_at(key, Instant::now())
    }
}

/// Redacts sensitive HTTP header values (`Authorization`, `x-api-key`, `Cookie`, `Set-Cookie`).
pub fn redact_header_value(header_name: &str, value: &str) -> String {
    let lower = header_name.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "authorization" | "x-api-key" | "proxy-authorization" | "cookie" | "set-cookie"
    ) {
        return "<redacted>".to_string();
    }
    redact_sensitive_text(value)
}

/// Redacts common secret patterns (Bearer tokens, API key prefixes, sensitive header lines,
/// and `?key=` / `&key=` query parameters) from log lines and error messages.
pub fn redact_sensitive_text(input: &str) -> String {
    let mut out = redact_header_inline(input, "authorization:");
    out = redact_header_inline(&out, "x-api-key:");
    out = redact_prefixed_tokens(&out, "bearer ", true);
    out = redact_prefixed_tokens(&out, "sk-", false);
    out = redact_prefixed_tokens(&out, "AIza", false);
    out = redact_prefixed_tokens(&out, "tsl_", false);
    out = redact_query_param(&out, "key=");
    out = redact_query_param(&out, "api_key=");
    out
}

fn redact_header_inline(input: &str, header_prefix_lower: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    let plen = header_prefix_lower.len();
    while i < bytes.len() {
        if i + plen <= bytes.len()
            && input
                .get(i..i + plen)
                .is_some_and(|s| s.eq_ignore_ascii_case(header_prefix_lower))
        {
            out.push_str(&input[i..i + plen]);
            out.push_str(" <redacted>");
            i += plen;
            while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' && bytes[i] != b',' {
                i += 1;
            }
            continue;
        }
        if let Some(ch) = input[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn redact_query_param(input: &str, param_prefix: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    let plen = param_prefix.len();
    while i < bytes.len() {
        let preceded_by_delim = i > 0 && matches!(bytes[i - 1], b'?' | b'&');
        if preceded_by_delim
            && i + plen <= bytes.len()
            && input
                .get(i..i + plen)
                .is_some_and(|s| s.eq_ignore_ascii_case(param_prefix))
        {
            out.push_str(&input[i..i + plen]);
            out.push_str("<redacted>");
            i += plen;
            while i < bytes.len()
                && !bytes[i].is_ascii_whitespace()
                && !matches!(bytes[i], b'&' | b'"' | b'\'' | b')' | b'>' | b'#')
            {
                i += 1;
            }
            continue;
        }
        if let Some(ch) = input[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'+' | b'/' | b'=')
}

pub(crate) fn redact_prefixed_tokens(input: &str, prefix: &str, case_insensitive: bool) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let bytes = input.as_bytes();
    let plen = prefix.len();
    while i < input.len() {
        let at_boundary = i == 0 || (!bytes[i - 1].is_ascii_alphanumeric() && bytes[i - 1] != b'_');
        let matched = if at_boundary && i + plen <= input.len() && input.is_char_boundary(i + plen)
        {
            let slice = &input[i..i + plen];
            if case_insensitive {
                slice.eq_ignore_ascii_case(prefix)
            } else {
                slice == prefix
            }
        } else {
            false
        };
        if matched {
            let mut end = i + plen;
            while end < bytes.len() && is_token_char(bytes[end]) {
                end += 1;
            }
            if end > i + plen {
                if prefix.ends_with(' ') {
                    out.push_str(&input[i..i + plen]);
                    out.push_str("<redacted>");
                } else {
                    out.push_str("<redacted>");
                }
                i = end;
                continue;
            }
        }
        if let Some(ch) = input[i..].chars().next() {
            out.push(ch);
            i += ch.len_utf8();
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_headers_tokens_and_query_keys() {
        assert_eq!(
            redact_header_value("Authorization", "Bearer tsl_secret123"),
            "<redacted>"
        );
        assert_eq!(
            redact_header_value("x-api-key", "sk-ant-12345"),
            "<redacted>"
        );
        assert_eq!(
            redact_header_value("Content-Type", "application/json"),
            "application/json"
        );

        let raw = "Authorization: Bearer tsl_abcdef123456\nx-api-key: sk-proj-9999\nhttps://example.com?key=AIzaSySecret&foo=1";
        let clean = redact_sensitive_text(raw);
        assert!(!clean.contains("tsl_abcdef123456"));
        assert!(!clean.contains("sk-proj-9999"));
        assert!(!clean.contains("AIzaSySecret"));
        assert!(clean.contains("foo=1"));
    }

    #[test]
    fn rate_limiter_suppresses_storms_and_reports_count_on_next_window() {
        let limiter = LogRateLimiter::new(Duration::from_secs(10), 2);
        let t0 = Instant::now();
        assert_eq!(
            limiter.check_at("auth_fail", t0),
            LogDecision::Emit { suppressed: 0 }
        );
        assert_eq!(
            limiter.check_at("auth_fail", t0),
            LogDecision::Emit { suppressed: 0 }
        );
        assert_eq!(limiter.check_at("auth_fail", t0), LogDecision::Suppress);
        assert_eq!(limiter.check_at("auth_fail", t0), LogDecision::Suppress);
        let t1 = t0 + Duration::from_secs(11);
        assert_eq!(
            limiter.check_at("auth_fail", t1),
            LogDecision::Emit { suppressed: 2 }
        );
    }
}
