use herdr_relay::{identity::Identity, relay::Relay, retry::Backoff, service};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

async fn failed_gateway(
    status: u16,
    body: String,
    retry_after: u64,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut bytes = [0u8; 2048];
            let n = socket.read(&mut bytes).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&bytes[..n]);
            if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap_or("0")
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            assert!(request.len() < 16384);
        }
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nRetry-After: {retry_after}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    });
    (address, task)
}

fn profile() -> tempfile::TempDir {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    Identity::initialize(root.path()).unwrap();
    root
}

#[tokio::test]
async fn gateway_failures_remain_retryable_when_the_error_body_is_unfamiliar_or_oversized() {
    let root = profile();
    for (status, body) in [
        (503, r#"{"code":"UPSTREAM_UNAVAILABLE"}"#.to_string()),
        (502, "x".repeat(9000)),
        (408, r#"{"code":"REQUEST_TIMEOUT"}"#.to_string()),
    ] {
        let (address, gateway) = failed_gateway(status, body, 120).await;
        let relay = Relay::new(root.path(), "https://relay.example", Some(&address)).unwrap();
        let error = relay.operator_status().await.unwrap_err();
        gateway.await.unwrap();
        let delay = Backoff::default()
            .next(&error, 0)
            .expect("temporary HTTP failure must retry");
        assert!(delay >= Duration::from_secs(120));
    }
}

#[tokio::test]
async fn quota_cooldown_keeps_the_local_daemon_responsive_and_cancellable() {
    let root = profile();
    let (address, gateway) =
        failed_gateway(429, r#"{"code":"GLOBAL_QUOTA_EXCEEDED"}"#.into(), 86400).await;
    let relay = Relay::new(root.path(), "https://relay.example", Some(&address)).unwrap();
    let profile_path = root.path().to_owned();
    let mut daemon = tokio::spawn(async move {
        service::run(&profile_path, "https://relay.example", Some(relay)).await
    });
    gateway.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut daemon)
            .await
            .is_err()
    );
    let (origin, online, _) = service::status(root.path()).await.unwrap();
    assert_eq!(origin, "https://relay.example");
    assert!(!online);
    daemon.abort();
    assert!(daemon.await.unwrap_err().is_cancelled());
}

#[tokio::test]
async fn endpoints_advertise_their_stream_capabilities_to_the_relay() {
    let root = profile();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let gateway = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        while !request.windows(4).any(|v| v == b"\r\n\r\n") {
            let mut bytes = [0u8; 2048];
            let n = socket.read(&mut bytes).await.unwrap();
            assert!(n > 0);
            request.extend_from_slice(&bytes[..n]);
        }
        let body = r#"{"code":"RELAY_BUSY"}"#;
        let response = format!(
            "HTTP/1.1 503 Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8_lossy(&request).to_ascii_lowercase()
    });
    let relay = Relay::new(root.path(), "https://relay.example", Some(&address)).unwrap();
    let _ = relay.operator_status().await;
    let request = gateway.await.unwrap();
    assert!(
        request.contains("\r\nx-relay-features: stream-v2"),
        "{request}"
    );
}
