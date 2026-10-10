use herdr_relay::{
    control::{self, Context, Purpose},
    host,
    identity::Identity,
    pairing::{self, Store},
    secure,
    targets::{Locator, Registry},
};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::net::UnixListener;

#[tokio::test]
async fn authorized_attach_reserves_before_native_start_and_releases_on_disconnect() {
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let a = Identity::initialize(&temp.path().join("a")).unwrap();
    let b = Identity::initialize(&temp.path().join("b")).unwrap();
    let socket = temp.path().join("native.sock");
    let native = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let targets = herdr_relay::targets::Store::open(&temp.path().join("a")).unwrap();
    targets
        .add(
            "work".into(),
            Locator::Socket {
                path: socket.clone(),
            },
        )
        .unwrap();
    let registry = Registry::open(&temp.path().join("a")).unwrap();
    let target = registry.snapshot()[0].clone();
    let store = Store::open(&temp.path().join("a"), "https://relay.example").unwrap();
    let invitation = store.invite(pairing::now()).unwrap();
    store
        .redeem(
            &b.certificate,
            &pairing::RedeemRequest {
                invitation_id: invitation.invitation_id,
                secret: invitation.secret,
                request_id: "11".repeat(16),
            },
            pairing::now(),
        )
        .unwrap();
    let context = Context {
        version: 1,
        origin: "https://relay.example".into(),
        publisher: a.id.clone(),
        connector: b.id.clone(),
        purpose: Purpose::Attach,
        grant_version: Some(1),
        target: Some(target.id.clone()),
        channel_id: "22".repeat(16),
        boot_id: "33".repeat(16),
    };
    let (a_stream, b_stream) = tokio::io::duplex(4096);
    let profile = temp.path().join("a");
    let expected = context.clone();
    let shared = target.clone();
    let task =
        tokio::spawn(async move { host::serve(a_stream, &profile, &expected, &[shared]).await });
    let mut stream = secure::connect(b_stream, &b, &a.certificate, &context)
        .await
        .unwrap();
    let ready: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(ready["result"], "ready");
    assert_eq!(target.status().status, "busy");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), native.accept())
            .await
            .is_err(),
        "native socket opened before connector was ready"
    );
    control::send(
        &mut stream,
        &serde_json::json!({"runtime_id":ready["runtime_id"]}),
    )
    .await
    .unwrap();
    let started: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(started["result"], "started");
    let (mut native, _) = native.accept().await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(b"native hello and following bytes")
        .await
        .unwrap();
    let mut bytes = [0; 32];
    native.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"native hello and following bytes");
    // Replace the path while the old socket is still connected. Registry
    // refresh must close the old native stream before publishing the new runtime.
    std::fs::remove_file(&socket).unwrap();
    let _replacement = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    tokio::time::timeout(Duration::from_millis(500), registry.reload())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(100), native.read(&mut bytes))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    assert_eq!(task.await.unwrap().unwrap_err().code, "TARGET_DISABLED");
    let next = registry.snapshot()[0].clone();
    assert_ne!(target.status().runtime_id, next.status().runtime_id);
    assert_eq!(next.status().status, "online");
    drop(stream);
    // A crashed listener can leave the socket pathname behind. A failed
    // authorized native open must be reflected as offline, not online.
    drop(_replacement);
    let (host_side, client_side) = tokio::io::duplex(4096);
    let saved_context = context.clone();
    let saved_target = next.clone();
    let profile = temp.path().join("a");
    let failed = tokio::spawn(async move {
        host::serve(host_side, &profile, &saved_context, &[saved_target]).await
    });
    let mut client = secure::connect(client_side, &b, &a.certificate, &context)
        .await
        .unwrap();
    let ready: serde_json::Value = control::receive(&mut client).await.unwrap();
    control::send(
        &mut client,
        &serde_json::json!({"runtime_id":ready["runtime_id"]}),
    )
    .await
    .unwrap();
    let rejected: serde_json::Value = control::receive(&mut client).await.unwrap();
    assert_eq!(rejected["code"], "TARGET_OFFLINE");
    control::send(&mut client, &serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(failed.await.unwrap().unwrap_err().code, "TARGET_OFFLINE");
    assert_eq!(next.status().status, "offline");
    // A silent peer in TLS setup cannot delay disabling or generation turnover.
    let (pending, _silent) = tokio::io::duplex(4096);
    let profile = temp.path().join("a");
    let waiting =
        tokio::spawn(async move { host::serve(pending, &profile, &context, &[next]).await });
    tokio::task::yield_now().await;
    targets.set_enabled(&target.id, false).unwrap();
    tokio::time::timeout(Duration::from_millis(500), registry.reload())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(waiting.await.unwrap().unwrap_err().code, "TARGET_DISABLED");
    assert!(registry.snapshot().is_empty());
}

#[tokio::test]
async fn closing_one_native_client_does_not_make_the_shared_session_offline() {
    let temp = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let a = Identity::initialize(&temp.path().join("a")).unwrap();
    let b = Identity::initialize(&temp.path().join("b")).unwrap();
    let socket = temp.path().join("native.sock");
    let native = UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let targets = herdr_relay::targets::Store::open(&temp.path().join("a")).unwrap();
    targets
        .add(
            "work".into(),
            Locator::Socket {
                path: socket.clone(),
            },
        )
        .unwrap();
    let registry = Registry::open(&temp.path().join("a")).unwrap();
    let target = registry.snapshot()[0].clone();
    let store = Store::open(&temp.path().join("a"), "https://relay.example").unwrap();
    let invitation = store.invite(pairing::now()).unwrap();
    store
        .redeem(
            &b.certificate,
            &pairing::RedeemRequest {
                invitation_id: invitation.invitation_id,
                secret: invitation.secret,
                request_id: "11".repeat(16),
            },
            pairing::now(),
        )
        .unwrap();
    let context = Context {
        version: 1,
        origin: "https://relay.example".into(),
        publisher: a.id.clone(),
        connector: b.id.clone(),
        purpose: Purpose::Attach,
        grant_version: Some(1),
        target: Some(target.id.clone()),
        channel_id: "22".repeat(16),
        boot_id: "33".repeat(16),
    };
    let (a_stream, b_stream) = tokio::io::duplex(4096);
    let profile = temp.path().join("a");
    let expected = context.clone();
    let shared = target.clone();
    let task =
        tokio::spawn(async move { host::serve(a_stream, &profile, &expected, &[shared]).await });
    let mut stream = secure::connect(b_stream, &b, &a.certificate, &context)
        .await
        .unwrap();
    let ready: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(ready["result"], "ready");
    assert_eq!(target.status().status, "busy");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), native.accept())
            .await
            .is_err(),
        "native socket opened before connector was ready"
    );
    control::send(
        &mut stream,
        &serde_json::json!({"runtime_id":ready["runtime_id"]}),
    )
    .await
    .unwrap();
    let started: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(started["result"], "started");
    let (mut native, _) = native.accept().await.unwrap();
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(b"native hello and following bytes")
        .await
        .unwrap();
    let mut bytes = [0; 32];
    native.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"native hello and following bytes");
    native.write_all(b"hello").await.unwrap();
    native.flush().await.unwrap();
    let mut reply = [0; 5];
    stream.read_exact(&mut reply).await.unwrap();
    drop(native);
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        target.status().status,
        "online",
        "closing one attached native socket is not server shutdown"
    );
    registry.reload().await.unwrap();
    assert_eq!(registry.snapshot()[0].status().status, "online");
}
