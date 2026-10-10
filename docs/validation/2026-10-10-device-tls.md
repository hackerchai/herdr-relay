# Device identity and local TLS acceptance

2026-10-10, macOS arm64, Rust/Cargo 1.99.0. Covers issue #2 on this host.

The CLI initialized two separate private profiles, reloaded the same public
fingerprint in a new process, and completed `identity diagnose --peer-profile`
over an anonymous Unix socket pair. The diagnostic opens no network listener
or remote connection and exchanges only a fixed diagnostic payload.

The locked implementation uses rustls 0.23.45, tokio-rustls 0.26.6, ring 0.17.14,
rustls-webpki 0.103.15 and rcgen 0.14.10. Identity is an Ed25519 PKCS#8 key;
device ID is lowercase SHA-256 of DER SPKI. The self-signed leaf has the DNS SAN
`device.herdr.invalid`, digitalSignature usage, serverAuth/clientAuth EKUs, a
five-minute clock allowance and ten-year validity. Changing the identity key
requires pairing again. An expired certificate fails validation.

Both endpoint roles pin the peer SPKI, reject intermediates and non-Ed25519
keys, and delegate certificate and CertificateVerify validation to WebPKI.
Production configurations offer TLS 1.3 only with ALPN `herdr-relay/1`.
Session caches/tickets and 0-RTT are disabled; TLS 1.2 is compiled solely to
exercise a negative downgrade test. Certificate files and control records
are capped at 8 KiB, TLS buffers at 64 KiB and records at 16 KiB. Handshake and
context exchange each have a ten-second deadline; the entire diagnostic has
a thirty-second deadline.

Necessary module tests passed for:

- Persistent, isolated identity, private file modes, symlink rejection and
  refusing to silently replace a partially missing identity.
- Real mutual TLS 1.3, unrelated keys in either role, forged signatures in
  either role, malformed certificates and TLS 1.2 downgrade.
- Repeated full handshakes and unavailable early data before handshake.
- Origin, role, purpose, target, channel and boot-context mismatch rejection;
  exact framing preserves bytes following BIND.

`cargo test --locked`, formatting and Clippy with warnings denied passed.
No application e2e tests were added. Cross-platform runtime testing, remote
relay testing and pairing transactions belong to subsequent tickets; this
record does not claim those checks or an independent security audit.
