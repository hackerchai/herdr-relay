use crate::{
    Error, Result,
    identity::{Identity, certificate_spki},
};
use rustls::{
    ClientConfig, DigitallySignedStruct, DistinguishedName, RootCertStore, ServerConfig,
    SignatureScheme,
    client::{
        WebPkiServerVerifier,
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    },
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::{
        WebPkiClientVerifier,
        danger::{ClientCertVerified, ClientCertVerifier},
    },
};
use std::sync::Arc;

pub fn client_config(
    identity: &Identity,
    peer: &CertificateDer<'static>,
) -> Result<Arc<ClientConfig>> {
    let mut config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(tls_error)?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedVerifier::new(peer)?))
            .with_client_auth_cert(
                vec![identity.certificate.clone()],
                identity.private_key.clone_key(),
            )
            .map_err(tls_error)?;
    config.resumption = rustls::client::Resumption::disabled();
    config.enable_early_data = false;
    config.max_fragment_size = Some(16 * 1024);
    config.alpn_protocols = vec![b"herdr-relay/1".to_vec()];
    Ok(Arc::new(config))
}
pub fn server_config(
    identity: &Identity,
    peer: &CertificateDer<'static>,
) -> Result<Arc<ServerConfig>> {
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(tls_error)?
            .with_client_cert_verifier(Arc::new(PinnedVerifier::new(peer)?))
            .with_single_cert(
                vec![identity.certificate.clone()],
                identity.private_key.clone_key(),
            )
            .map_err(tls_error)?;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.max_fragment_size = Some(16 * 1024);
    config.alpn_protocols = vec![b"herdr-relay/1".to_vec()];
    Ok(Arc::new(config))
}

#[derive(Debug)]
struct PinnedVerifier {
    spki: Vec<u8>,
    server: Arc<WebPkiServerVerifier>,
    client: Arc<dyn ClientCertVerifier>,
}

impl PinnedVerifier {
    fn new(peer: &CertificateDer<'static>) -> Result<Self> {
        let spki = certificate_spki(peer)?;
        let mut roots = RootCertStore::empty();
        roots.add(peer.clone()).map_err(tls_error)?;
        let roots = Arc::new(roots);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let server = WebPkiServerVerifier::builder_with_provider(roots.clone(), provider.clone())
            .build()
            .map_err(|_| Error::new("INVALID_CERTIFICATE", "invalid pinned server certificate"))?;
        let client = WebPkiClientVerifier::builder_with_provider(roots, provider)
            .build()
            .map_err(|_| Error::new("INVALID_CERTIFICATE", "invalid pinned client certificate"))?;
        Ok(Self {
            spki,
            server,
            client,
        })
    }

    fn check_pin(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
    ) -> std::result::Result<(), rustls::Error> {
        if cert.len() > 8192 || !intermediates.is_empty() {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            ));
        }
        let actual = certificate_spki(cert).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        if actual != self.spki {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ));
        }
        Ok(())
    }
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        self.check_pin(cert, intermediates)?;
        self.server
            .verify_server_cert(cert, intermediates, name, ocsp, now)
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 is required".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.server.verify_tls13_signature(message, cert, signature)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

impl ClientCertVerifier for PinnedVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        self.client.root_hint_subjects()
    }
    fn verify_client_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        self.check_pin(cert, intermediates)?;
        self.client.verify_client_cert(cert, intermediates, now)
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 is required".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        self.client.verify_tls13_signature(message, cert, signature)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

fn tls_error(e: rustls::Error) -> Error {
    Error::new("TLS_AUTH_FAILED", e.to_string())
}

fn tls_io(error: std::io::Error) -> Error {
    let auth = error.get_ref().is_some_and(|e| e.is::<rustls::Error>())
        || error.kind() == std::io::ErrorKind::InvalidData;
    Error::new(
        if auth {
            "TLS_AUTH_FAILED"
        } else {
            "CONNECTION_LOST"
        },
        error.to_string(),
    )
}

pub async fn connect<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: S,
    identity: &Identity,
    peer: &CertificateDer<'static>,
    context: &crate::control::Context,
) -> Result<tokio_rustls::client::TlsStream<S>> {
    check_context(identity, peer, context, false)?;
    let connector = tokio_rustls::TlsConnector::from(client_config(identity, peer)?);
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        connector.connect_with(
            crate::identity::DEVICE_NAME.try_into().unwrap(),
            stream,
            |connection| connection.set_buffer_limit(Some(64 * 1024)),
        ),
    )
    .await
    .map_err(|_| Error::new("TLS_TIMEOUT", "device TLS handshake exceeded 10 seconds"))?
    .map_err(tls_io)?;
    crate::timing::mark("tls");
    crate::control::bind(&mut stream, context).await?;
    crate::timing::mark("bind");
    Ok(stream)
}

pub async fn accept<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: S,
    identity: &Identity,
    peer: &CertificateDer<'static>,
    context: &crate::control::Context,
) -> Result<tokio_rustls::server::TlsStream<S>> {
    check_context(identity, peer, context, true)?;
    let acceptor = tokio_rustls::TlsAcceptor::from(server_config(identity, peer)?);
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        acceptor.accept_with(stream, |connection| {
            connection.set_buffer_limit(Some(64 * 1024))
        }),
    )
    .await
    .map_err(|_| Error::new("TLS_TIMEOUT", "device TLS handshake exceeded 10 seconds"))?
    .map_err(tls_io)?;
    crate::timing::mark("tls");
    crate::control::bind(&mut stream, context).await?;
    crate::timing::mark("bind");
    Ok(stream)
}

/// Direct LAN entry: accept only a client whose certificate a current grant
/// pins. TLS 1.3 encrypts the client certificate, so no device identity is
/// visible on the local network. Returns the authenticated certificate.
pub(crate) async fn accept_granted<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: S,
    identity: &Identity,
    granted: &[CertificateDer<'static>],
) -> Result<(tokio_rustls::server::TlsStream<S>, CertificateDer<'static>)> {
    let pins = granted
        .iter()
        .map(PinnedVerifier::new)
        .collect::<Result<Vec<_>>>()?;
    if pins.is_empty() {
        return Err(Error::new(
            "NOT_AUTHORIZED",
            "no device may access this host",
        ));
    }
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(tls_error)?
            .with_client_cert_verifier(Arc::new(GrantedVerifier { pins }))
            .with_single_cert(
                vec![identity.certificate.clone()],
                identity.private_key.clone_key(),
            )
            .map_err(tls_error)?;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.max_fragment_size = Some(16 * 1024);
    config.alpn_protocols = vec![b"herdr-relay/1".to_vec()];
    let stream = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept_with(stream, |c| c.set_buffer_limit(Some(64 * 1024))),
    )
    .await
    .map_err(|_| Error::new("TLS_TIMEOUT", "device TLS handshake exceeded 10 seconds"))?
    .map_err(tls_io)?;
    crate::timing::mark("tls");
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| Error::new("TLS_AUTH_FAILED", "client certificate required"))?
        .clone()
        .into_owned();
    Ok((stream, certificate))
}

/// Accepts a client certificate pinned by any one current grant.
#[derive(Debug)]
struct GrantedVerifier {
    pins: Vec<PinnedVerifier>,
}
impl ClientCertVerifier for GrantedVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        self.pins
            .iter()
            .find_map(|pin| pin.verify_client_cert(cert, intermediates, now).ok())
            .ok_or(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 is required".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}

fn check_context(
    identity: &Identity,
    peer: &CertificateDer<'static>,
    context: &crate::control::Context,
    publisher: bool,
) -> Result<()> {
    use sha2::{Digest, Sha256};
    context.validate()?;
    let peer_id = hex::encode(Sha256::digest(certificate_spki(peer)?));
    let (own, other) = if publisher {
        (&context.publisher, &context.connector)
    } else {
        (&context.connector, &context.publisher)
    };
    if own != &identity.id || other != &peer_id {
        return Err(Error::new(
            "CONTEXT_MISMATCH",
            "channel roles do not match authenticated device identities",
        ));
    }
    Ok(())
}

/// The only unpaired entry point. Its caller must expose pairing redemption
/// only, never a normal control or attach stream.
pub(crate) async fn accept_pair<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: S,
    identity: &Identity,
    origin: &str,
    channel: &str,
    boot: &str,
) -> Result<(tokio_rustls::server::TlsStream<S>, CertificateDer<'static>)> {
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(tls_error)?
            .with_client_cert_verifier(Arc::new(PairingVerifier))
            .with_single_cert(
                vec![identity.certificate.clone()],
                identity.private_key.clone_key(),
            )
            .map_err(tls_error)?;
    config.session_storage = Arc::new(rustls::server::NoServerSessionStorage {});
    config.send_tls13_tickets = 0;
    config.max_early_data_size = 0;
    config.max_fragment_size = Some(16 * 1024);
    config.alpn_protocols = vec![b"herdr-relay/1".to_vec()];
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept_with(stream, |c| c.set_buffer_limit(Some(64 * 1024))),
    )
    .await
    .map_err(|_| Error::new("TLS_TIMEOUT", "pairing handshake exceeded 10 seconds"))?
    .map_err(tls_io)?;
    let certificate = stream
        .get_ref()
        .1
        .peer_certificates()
        .and_then(|certs| certs.first())
        .ok_or_else(|| Error::new("TLS_AUTH_FAILED", "client certificate required"))?
        .clone()
        .into_owned();
    let context = crate::control::Context {
        version: 1,
        origin: origin.into(),
        publisher: identity.id.clone(),
        connector: crate::pairing::fingerprint(&certificate)?,
        purpose: crate::control::Purpose::Pair,
        grant_version: None,
        target: None,
        channel_id: channel.into(),
        boot_id: boot.into(),
    };
    crate::control::bind(&mut stream, &context).await?;
    Ok((stream, certificate))
}

#[derive(Debug)]
struct PairingVerifier;
impl ClientCertVerifier for PairingVerifier {
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self,
        cert: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> std::result::Result<ClientCertVerified, rustls::Error> {
        if cert.len() > 8192 || !intermediates.is_empty() {
            return Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::BadEncoding,
            ));
        }
        // Trust introduction is the one-time secret, but key possession,
        // Ed25519 structure, validity and clientAuth still require verification.
        let verifier = PinnedVerifier::new(&cert.clone().into_owned()).map_err(|_| {
            rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding)
        })?;
        verifier.verify_client_cert(cert, intermediates, now)
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        Err(rustls::Error::General("TLS 1.3 is required".into()))
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signature: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![SignatureScheme::ED25519]
    }
}
