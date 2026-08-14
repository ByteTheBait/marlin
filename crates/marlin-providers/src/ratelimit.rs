use std::time::{Duration, SystemTime};

use reqwest::header::HeaderMap;

use super::RateLimitState;

pub fn parse_rate_limit_state(headers: &HeaderMap) -> Option<RateLimitState> {
    let mut state = RateLimitState {
        remaining_requests: -1,
        remaining_tokens: -1,
        reset_requests_at: None,
        reset_tokens_at: None,
    };
    let mut found = false;

    for h in &[
        "x-ratelimit-remaining-requests",
        "anthropic-ratelimit-requests-remaining",
        "ratelimit-remaining-requests",
    ] {
        if let Some(v) = headers.get(*h).and_then(|v| v.to_str().ok()) {
            if let Ok(n) = v.parse::<i64>() {
                state.remaining_requests = n;
                found = true;
                break;
            }
        }
    }

    for h in &[
        "x-ratelimit-remaining-tokens",
        "anthropic-ratelimit-tokens-remaining",
        "ratelimit-remaining-tokens",
    ] {
        if let Some(v) = headers.get(*h).and_then(|v| v.to_str().ok()) {
            if let Ok(n) = v.parse::<i64>() {
                state.remaining_tokens = n;
                found = true;
                break;
            }
        }
    }

    for h in &[
        "x-ratelimit-reset-requests",
        "anthropic-ratelimit-requests-reset",
    ] {
        if let Some(v) = headers.get(*h).and_then(|v| v.to_str().ok()) {
            if let Some(secs) = parse_wait_seconds(v) {
                state.reset_requests_at = Some(SystemTime::now() + Duration::from_secs(secs));
                break;
            }
        }
    }

    for h in &[
        "x-ratelimit-reset-tokens",
        "anthropic-ratelimit-tokens-reset",
    ] {
        if let Some(v) = headers.get(*h).and_then(|v| v.to_str().ok()) {
            if let Some(secs) = parse_wait_seconds(v) {
                state.reset_tokens_at = Some(SystemTime::now() + Duration::from_secs(secs));
                break;
            }
        }
    }

    if found {
        Some(state)
    } else {
        None
    }
}

pub fn retry_after_seconds(headers: &HeaderMap, default_secs: u32) -> u32 {
    if let Some(v) = headers.get("retry-after").and_then(|v| v.to_str().ok()) {
        if let Some(s) = parse_wait_seconds(v) {
            return s as u32;
        }
    }

    let reset_headers = [
        "x-ratelimit-reset-requests",
        "x-ratelimit-reset-tokens",
        "x-ratelimit-reset",
        "anthropic-ratelimit-requests-reset",
        "anthropic-ratelimit-tokens-reset",
        "ratelimit-reset",
    ];
    let mut best = 0u64;
    for h in &reset_headers {
        if let Some(v) = headers.get(*h).and_then(|v| v.to_str().ok()) {
            if let Some(s) = parse_wait_seconds(v) {
                if s > best {
                    best = s;
                }
            }
        }
    }
    if best > 0 {
        return best as u32;
    }

    default_secs
}

fn parse_wait_seconds(val: &str) -> Option<u64> {
    // Plain integer: seconds or Unix timestamp
    if let Ok(n) = val.parse::<u64>() {
        if n > 1_000_000_000 {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            return if n > now { Some(n - now) } else { None };
        }
        return if n > 0 { Some(n) } else { None };
    }

    // RFC3339 / ISO8601
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(val) {
        let then = t.timestamp() as u64;
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        return if then > now { Some(then - now) } else { None };
    }

    None
}
