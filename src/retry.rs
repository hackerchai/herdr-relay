use crate::Error;
use std::time::{Duration, SystemTime};

#[derive(Default)]
pub struct Backoff {
    failures: u32,
}

impl Backoff {
    pub fn reset(&mut self) {
        self.failures = 0;
    }
    pub fn next(&mut self, error: &Error, jitter: u64) -> Option<Duration> {
        if !matches!(
            error.code,
            "NETWORK_LOST"
                | "CONNECTION_LOST"
                | "HOST_OFFLINE"
                | "TARGET_OFFLINE"
                | "TARGET_CHANGED"
                | "HANDSHAKE_TIMEOUT"
                | "HANDSHAKE_FAILED"
                | "TLS_TIMEOUT"
                | "BIND_TIMEOUT"
                | "CONTROL_TIMEOUT"
                | "RELAY_BUSY"
                | "RATE_LIMITED"
                | "GLOBAL_QUOTA_EXCEEDED"
                | "CLOCK_SKEW"
                | "INTERNAL_ERROR"
                | "TICKET_INVALID"
                | "CHANNEL_EXPIRED"
        ) {
            return None;
        }
        let ceiling = (1000u64 << self.failures.min(5)).min(30_000);
        self.failures = self.failures.saturating_add(1);
        let delay = Duration::from_millis(ceiling / 2 + jitter % (ceiling / 2 + 1));
        // Server cooldowns are a floor, not a shared wake-up instant for every
        // host. Keep jitter after Retry-After to avoid a midnight retry herd.
        let floor = error.retry_after.unwrap_or_else(|| {
            if matches!(
                error.code,
                "GLOBAL_QUOTA_EXCEEDED" | "RATE_LIMITED" | "CLOCK_SKEW"
            ) {
                Duration::from_secs(60)
            } else {
                Duration::ZERO
            }
        });
        Some(floor.saturating_add(delay))
    }
}

pub fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return value.parse().ok().map(Duration::from_secs);
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|date| date.duration_since(now).unwrap_or_default())
}

pub(crate) fn jitter() -> crate::Result<u64> {
    let id = crate::control::random_id()?;
    Ok(u64::from_str_radix(&id[..16], 16).expect("random ID is hexadecimal"))
}

pub(crate) async fn wait(mut duration: Duration) {
    // Even an enormous syntactically valid Retry-After must remain
    // cancellable and cannot overflow Tokio's platform Instant range.
    while !duration.is_zero() {
        let part = duration.min(Duration::from_secs(3600));
        tokio::time::sleep(part).await;
        duration -= part;
    }
}
