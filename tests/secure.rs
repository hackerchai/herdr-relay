use herdr_relay::{
    identity::{DEVICE_NAME, Identity},
    secure::{client_config, server_config},
};
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn device() -> (tempfile::TempDir, Identity) {
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let identity = Identity::initialize(dir.path()).unwrap();
    (dir, identity)
}

#[tokio::test]
async fn idle_native_source_flushes_the_entire_tls_frame_without_client_input() {
    let (_a, a) = device();
    let (_b, b) = device();
    let (wire_a, wire_b) = tokio::io::duplex(1024);
    let acceptor = tokio_rustls::TlsAcceptor::from(server_config(&a, &b.certificate).unwrap());
    let connector = tokio_rustls::TlsConnector::from(client_config(&b, &a.certificate).unwrap());
    let (server, client) = tokio::join!(
        acceptor.accept(wire_a),
        connector.connect(DEVICE_NAME.try_into().unwrap(), wire_b)
    );
    let mut client = client.unwrap();
    let (bridge_side, mut native) = tokio::io::duplex(4096);
    let bridge = tokio::spawn(herdr_relay::transport::bridge_native(
        server.unwrap(),
        bridge_side,
        Duration::from_secs(1),
    ));
    let writer = tokio::spawn(async move {
        native.write_all(&vec![0x79; 100003]).await.unwrap();
        native // Keep the source open and idle, as a completed Herdr screen is.
    });
    let mut received = vec![0; 100003];
    tokio::time::timeout(Duration::from_secs(2), client.read_exact(&mut received))
        .await
        .expect("a passive viewer must receive the final TLS bytes without sending a key")
        .unwrap();
    assert!(received.iter().all(|b| *b == 0x79));
    drop(writer.await.unwrap());
    bridge.await.unwrap().unwrap();
}

#[tokio::test]
async fn an_unrelated_identity_is_rejected_in_either_role() {
    let (_a, a) = device();
    let (_b, b) = device();
    let (_c, c) = device();
    assert!(
        handshake(
            client_config(&b, &c.certificate).unwrap(),
            server_config(&a, &b.certificate).unwrap()
        )
        .await
        .is_err()
    );
    assert!(
        handshake(
            client_config(&b, &a.certificate).unwrap(),
            server_config(&a, &c.certificate).unwrap()
        )
        .await
        .is_err()
    );
}

#[derive(Debug)]
struct ForgedSignature;

impl rustls::sign::SigningKey for ForgedSignature {
    fn choose_scheme(
        &self,
        offered: &[rustls::SignatureScheme],
    ) -> Option<Box<dyn rustls::sign::Signer>> {
        offered
            .contains(&rustls::SignatureScheme::ED25519)
            .then(|| Box::new(Self) as Box<dyn rustls::sign::Signer>)
    }
    fn algorithm(&self) -> rustls::SignatureAlgorithm {
        rustls::SignatureAlgorithm::ED25519
    }
}

impl rustls::sign::Signer for ForgedSignature {
    fn sign(&self, _: &[u8]) -> Result<Vec<u8>, rustls::Error> {
        Ok(vec![0; 64])
    }
    fn scheme(&self) -> rustls::SignatureScheme {
        rustls::SignatureScheme::ED25519
    }
}

#[tokio::test]
async fn knowing_a_pinned_certificate_does_not_allow_forging_either_handshake_signature() {
    let (_a, a) = device();
    let (_b, b) = device();
    let client = client_config(&b, &a.certificate).unwrap();
    let server = server_config(&a, &b.certificate).unwrap();
    let mut forged_server = (*server).clone();
    forged_server.cert_resolver = Arc::new(rustls::sign::SingleCertAndKey::from(
        rustls::sign::CertifiedKey::new(vec![a.certificate.clone()], Arc::new(ForgedSignature)),
    ));
    let error = handshake(client.clone(), Arc::new(forged_server))
        .await
        .unwrap_err();
    assert!(
        matches!(
            error
                .get_ref()
                .and_then(|e| e.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature
            ))
        ),
        "{error}"
    );
    let mut forged_client = (*client).clone();
    forged_client.client_auth_cert_resolver = Arc::new(rustls::sign::SingleCertAndKey::from(
        rustls::sign::CertifiedKey::new(vec![b.certificate.clone()], Arc::new(ForgedSignature)),
    ));
    let error = handshake(Arc::new(forged_client), server)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error
                .get_ref()
                .and_then(|e| e.downcast_ref::<rustls::Error>()),
            Some(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadSignature
            ))
        ),
        "{error}"
    );
}

#[tokio::test]
async fn malformed_certificate_and_protocol_downgrade_are_rejected() {
    let (_a, a) = device();
    let (_b, b) = device();
    let mut server = (*server_config(&a, &b.certificate).unwrap()).clone();
    server.cert_resolver = Arc::new(rustls::sign::SingleCertAndKey::from(
        rustls::sign::CertifiedKey::new(vec![vec![0, 1, 2].into()], Arc::new(ForgedSignature)),
    ));
    assert!(
        handshake(client_config(&b, &a.certificate).unwrap(), Arc::new(server))
            .await
            .is_err()
    );
    let mut roots = rustls::RootCertStore::empty();
    roots.add(a.certificate.clone()).unwrap();
    let client = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12])
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    let error = handshake(Arc::new(client), server_config(&a, &b.certificate).unwrap())
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("Tls12NotOfferedOrEnabled")
            || error.to_string().contains("protocol version"),
        "{error}"
    );
}

#[tokio::test]
async fn repeated_connections_perform_full_handshakes_and_cannot_send_early_data() {
    let (_a, a) = device();
    let (_b, b) = device();
    let client_config = client_config(&b, &a.certificate).unwrap();
    let server_config = server_config(&a, &b.certificate).unwrap();
    for _ in 0..2 {
        let mut pending =
            rustls::ClientConnection::new(client_config.clone(), DEVICE_NAME.try_into().unwrap())
                .unwrap();
        assert!(pending.early_data().is_none());
        let (mut client, mut server) = handshake(client_config.clone(), server_config.clone())
            .await
            .unwrap();
        server.write_all(b"ready").await.unwrap();
        server.flush().await.unwrap();
        let mut response = [0; 5];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(&response, b"ready");
        assert_eq!(
            client.get_ref().1.handshake_kind(),
            Some(rustls::HandshakeKind::Full)
        );
        assert_eq!(
            server.get_ref().1.handshake_kind(),
            Some(rustls::HandshakeKind::Full)
        );
        assert!(client.get_mut().1.early_data().is_none());
    }
}

async fn handshake(
    client: Arc<rustls::ClientConfig>,
    server: Arc<rustls::ServerConfig>,
) -> std::io::Result<(
    tokio_rustls::client::TlsStream<tokio::io::DuplexStream>,
    tokio_rustls::server::TlsStream<tokio::io::DuplexStream>,
)> {
    let (a, b) = tokio::io::duplex(4096);
    let connector = tokio_rustls::TlsConnector::from(client);
    let acceptor = tokio_rustls::TlsAcceptor::from(server);
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::try_join!(
            connector.connect(DEVICE_NAME.try_into().unwrap(), a),
            acceptor.accept(b)
        )
    })
    .await
    .expect("handshake must terminate")
}

#[tokio::test]
async fn known_devices_authenticate_each_other_and_exchange_tls13_data() {
    let (_a, a) = device();
    let (_b, b) = device();
    let (mut client, mut server) = handshake(
        client_config(&b, &a.certificate).unwrap(),
        server_config(&a, &b.certificate).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(
        client.get_ref().1.protocol_version(),
        Some(rustls::ProtocolVersion::TLSv1_3)
    );
    client.write_all(b"authenticated diagnostic").await.unwrap();
    client.flush().await.unwrap();
    let mut message = [0; 24];
    server.read_exact(&mut message).await.unwrap();
    assert_eq!(&message, b"authenticated diagnostic");
}

#[tokio::test]
async fn authenticated_channel_binds_context_and_preserves_following_bytes() {
    use herdr_relay::control::{Context, Purpose, bind};
    let (_a, a) = device();
    let (_b, b) = device();
    let context = Context {
        version: 1,
        origin: "https://relay.example".into(),
        publisher: a.id.clone(),
        connector: b.id.clone(),
        purpose: Purpose::Attach,
        grant_version: Some(1),
        target: Some("11".repeat(16)),
        channel_id: "22".repeat(16),
        boot_id: "33".repeat(16),
    };
    let (mut client, mut server) = handshake(
        client_config(&b, &a.certificate).unwrap(),
        server_config(&a, &b.certificate).unwrap(),
    )
    .await
    .unwrap();
    let send = async {
        bind(&mut client, &context).await.unwrap();
        client.write_all(b"native bytes after BIND").await.unwrap();
        client.flush().await.unwrap();
    };
    let receive = async {
        bind(&mut server, &context).await.unwrap();
        let mut bytes = [0; 23];
        server.read_exact(&mut bytes).await.unwrap();
        assert_eq!(&bytes, b"native bytes after BIND");
    };
    tokio::join!(send, receive);
}

#[tokio::test]
async fn secure_channel_rejects_context_cross_wiring_before_returning_a_stream() {
    use herdr_relay::{
        control::{Context, Purpose},
        secure,
    };
    let (_a, a) = device();
    let (_b, b) = device();
    let expected = Context {
        version: 1,
        origin: "https://relay.example".into(),
        publisher: a.id.clone(),
        connector: b.id.clone(),
        purpose: Purpose::Control,
        grant_version: Some(1),
        target: None,
        channel_id: "22".repeat(16),
        boot_id: "33".repeat(16),
    };
    let (c, s) = tokio::io::duplex(4096);
    let (client, server) = tokio::join!(
        secure::connect(c, &b, &a.certificate, &expected),
        secure::accept(s, &a, &b.certificate, &expected)
    );
    assert!(client.is_ok() && server.is_ok(), "{client:?} {server:?}");
    let mut contexts = vec![expected.clone(); 6];
    contexts[0].origin = "https://another-relay.example".into();
    contexts[1].purpose = Purpose::Pair;
    contexts[1].grant_version = None;
    contexts[2].purpose = Purpose::Attach;
    contexts[2].target = Some("44".repeat(16));
    contexts[3].channel_id = "55".repeat(16);
    contexts[4].boot_id = "66".repeat(16);
    contexts[5].grant_version = Some(2);
    for wrong in contexts {
        let (c, s) = tokio::io::duplex(4096);
        let (client, server) = tokio::join!(
            secure::connect(c, &b, &a.certificate, &wrong),
            secure::accept(s, &a, &b.certificate, &expected)
        );
        assert_eq!(client.unwrap_err().code, "CONTEXT_MISMATCH");
        assert_eq!(server.unwrap_err().code, "CONTEXT_MISMATCH");
    }
    let mut wrong_role = expected.clone();
    std::mem::swap(&mut wrong_role.publisher, &mut wrong_role.connector);
    let (c, _) = tokio::io::duplex(4096);
    assert_eq!(
        secure::connect(c, &b, &a.certificate, &wrong_role)
            .await
            .unwrap_err()
            .code,
        "CONTEXT_MISMATCH"
    );
}

#[tokio::test]
async fn pairing_entry_point_still_rejects_forged_client_signature() {
    let (a_dir, a) = device();
    let (_b_dir, b) = device();
    let host = herdr_relay::pairing::Store::open(a_dir.path(), "https://relay.example").unwrap();
    let mut config = (*client_config(&b, &a.certificate).unwrap()).clone();
    config.client_auth_cert_resolver = Arc::new(rustls::sign::SingleCertAndKey::from(
        rustls::sign::CertifiedKey::new(vec![b.certificate.clone()], Arc::new(ForgedSignature)),
    ));
    let (c, s) = tokio::io::duplex(4096);
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let channel = "11".repeat(16);
    let boot = "22".repeat(16);
    let (_, server) = tokio::join!(
        connector.connect(DEVICE_NAME.try_into().unwrap(), c),
        herdr_relay::pairing::serve(s, &host, &channel, &boot)
    );
    let error = server.unwrap_err();
    assert_eq!(error.code, "TLS_AUTH_FAILED");
    assert!(error.message.contains("BadSignature"), "{error}");
    assert!(host.grants().unwrap().allowed_here.is_empty());
}
