use herdr_relay::{
    Error,
    retry::{Backoff, retry_after},
};
use std::time::{Duration, SystemTime};

#[test]
fn recovery_is_bounded_jittered_and_never_waits_to_take_over_another_client() {
    let mut backoff = Backoff::default();
    let lost = Error::new("NETWORK_LOST", "offline");
    assert_eq!(backoff.next(&lost, 0), Some(Duration::from_millis(500)));
    assert_eq!(backoff.next(&lost, 1000), Some(Duration::from_secs(2)));
    for _ in 0..100 {
        let delay = backoff.next(&lost, u64::MAX).unwrap();
        assert!(delay >= Duration::from_millis(500) && delay <= Duration::from_secs(30));
    }
    backoff.reset();
    assert_eq!(backoff.next(&lost, 500), Some(Duration::from_secs(1)));
    for code in [
        "TARGET_BUSY",
        "QUOTA_EXCEEDED",
        "GLOBAL_QUOTA_EXCEEDED",
        "NOT_AUTHORIZED",
        "TLS_AUTH_FAILED",
        "IDENTITY_CHANGED",
        "INCOMPATIBLE_HERDR",
        "CANCELLED",
        "TARGET_NOT_SHARED",
        "PROTOCOL_ERROR",
        "NATIVE_CLIENT_FAILED",
    ] {
        assert_eq!(
            backoff.next(&Error::new(code, "terminal"), 0),
            None,
            "{code}"
        );
    }
    let mut limited = Error::new("RATE_LIMITED", "slow down");
    limited.retry_after = Some(Duration::from_secs(90));
    assert!(backoff.next(&limited, 0).unwrap() >= Duration::from_secs(90));
}

#[test]
fn rate_limit_headers_support_seconds_and_http_dates() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(784111777);
    assert_eq!(retry_after("120", now), Some(Duration::from_secs(120)));
    assert_eq!(
        retry_after("Sun, 06 Nov 1994 08:51:37 GMT", now),
        Some(Duration::from_secs(120))
    );
    assert_eq!(
        retry_after("Sun, 06 Nov 1994 08:47:37 GMT", now),
        Some(Duration::ZERO)
    );
    for bad in ["-1", "NaN", "1.5", ""] {
        assert_eq!(retry_after(bad, now), None);
    }
}
