use herdr_relay::{
    config::{Config, PUBLIC_ORIGIN, use_config},
    identity::Identity,
    relay::Relay,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "correct-horse-battery-staple-42";

/// One HTTP exchange: returns the request head and answers with `status`/`body`.
async fn answer_once(
    status: &str,
    body: &'static str,
) -> (String, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let status = status.to_owned();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        let mut byte = [0];
        while !head.ends_with(b"\r\n\r\n") {
            socket.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
        }
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(head).unwrap().to_ascii_lowercase()
    });
    (base, task)
}

#[tokio::test]
async fn tokens_reach_only_their_own_self_hosted_relay() {
    use std::os::unix::fs::PermissionsExt;
    let profile = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    Identity::initialize(profile.path()).unwrap();
    let mut config = Config::default();
    config
        .set("token", TOKEN, Some("https://relay.example.org"))
        .unwrap();
    use_config(config);

    // The configured relay receives its token and its rejection is reported.
    let (base, request) = answer_once("401 Unauthorized", r#"{"code":"RELAY_AUTH_INVALID"}"#).await;
    let relay = Relay::new(profile.path(), "https://relay.example.org", Some(&base)).unwrap();
    let error = relay.operator_status().await.unwrap_err();
    assert_eq!(error.code, "RELAY_AUTH_INVALID");
    assert!(error.message.contains("token"), "{}", error.message);
    let head = request.await.unwrap();
    assert!(
        head.contains(&format!(
            "authorization: bearer {}",
            TOKEN.to_ascii_lowercase()
        )),
        "{head}"
    );

    // Another relay and the public relay never see it.
    for origin in ["https://other.example.org", PUBLIC_ORIGIN] {
        let (base, request) =
            answer_once("401 Unauthorized", r#"{"code":"RELAY_AUTH_REQUIRED"}"#).await;
        let relay = Relay::new(profile.path(), origin, Some(&base)).unwrap();
        assert_eq!(
            relay.operator_status().await.unwrap_err().code,
            "RELAY_AUTH_REQUIRED"
        );
        let head = request.await.unwrap();
        assert!(!head.contains("authorization"), "{origin}: {head}");
    }
}

#[tokio::test]
async fn an_access_check_distinguishes_a_missing_token_from_an_open_relay() {
    use std::os::unix::fs::PermissionsExt;
    let profile = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    Identity::initialize(profile.path()).unwrap();
    let origin = "https://unconfigured.example.org";
    let (base, _) = answer_once("401 Unauthorized", r#"{"code":"RELAY_AUTH_REQUIRED"}"#).await;
    let relay = Relay::new(profile.path(), origin, Some(&base)).unwrap();
    assert_eq!(
        relay.check_access().await.unwrap_err().code,
        "RELAY_AUTH_REQUIRED"
    );
    // Any other answer means the request got past authorization.
    let (base, _) = answer_once("400 Bad Request", r#"{"code":"PROTOCOL_ERROR"}"#).await;
    let relay = Relay::new(profile.path(), origin, Some(&base)).unwrap();
    relay.check_access().await.unwrap();
}
