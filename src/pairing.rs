use crate::{
    Error, Result,
    control::random_id,
    identity::{Identity, certificate_spki, private_directory},
};
use rustls::pki_types::CertificateDer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};

const RETENTION: u64 = 86400;
const STATE_LIMIT: usize = 1024 * 1024;

// Deliberately no Debug: invitations and requests contain bearer secrets.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub version: u8,
    pub origin: String,
    pub publisher: String,
    pub certificate: String,
    pub invitation_id: String,
    pub secret: String,
    pub expires_at: u64,
}

impl Invitation {
    pub fn encode(&self) -> Result<String> {
        Ok(format!(
            "hrp1:{}",
            hex::encode(serde_json::to_vec(self).map_err(|_| invalid())?)
        ))
    }
    pub fn decode(value: &str) -> Result<Self> {
        if value.len() > 8192 {
            return Err(invalid());
        }
        let bytes =
            hex::decode(value.strip_prefix("hrp1:").ok_or_else(invalid)?).map_err(|_| invalid())?;
        let invitation: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        invitation.validate()?;
        Ok(invitation)
    }
    pub fn peer_certificate(&self) -> Result<CertificateDer<'static>> {
        Ok(hex::decode(&self.certificate)
            .map_err(|_| invalid())?
            .into())
    }
    fn validate(&self) -> Result<()> {
        validate_origin(&self.origin)?;
        if self.version != 1
            || !hex_id(&self.invitation_id, 32)
            || !hex_id(&self.secret, 64)
            || self.certificate.len() > 4096
            || fingerprint(&self.peer_certificate()?)? != self.publisher
        {
            return Err(invalid());
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemRequest {
    pub invitation_id: String,
    pub secret: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub publisher: String,
    pub connector: String,
    pub origin: String,
    pub invitation_id: String,
    pub request_id: String,
    pub grant_version: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub device_id: String,
    pub certificate: String,
    pub version: u64,
}

#[derive(Debug, Serialize)]
pub struct Grants {
    pub allowed_here: Vec<Grant>,
    pub accessible_hosts: Vec<Grant>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedInvitation {
    id: String,
    secret_hash: String,
    expires_at: u64,
    result: Option<Receipt>,
    retain_until: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pending {
    invitation_id: String,
    publisher: String,
    request_id: String,
    retain_until: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u8,
    device_id: String,
    origin: String,
    invitations: Vec<SavedInvitation>,
    incoming: Vec<Grant>,
    outgoing: Vec<Grant>,
    pending: Vec<Pending>,
    #[serde(default)]
    grant_serial: u64,
}

pub struct Store {
    profile: PathBuf,
    identity: Identity,
    origin: String,
}

impl Store {
    /// Explicit trust-scope reset. Neither identity nor shared targets move.
    pub fn switch_origin(profile: &Path, from: &str, to: &str) -> Result<()> {
        validate_origin(from)?;
        validate_origin(to)?;
        let _daemon = crate::service::lock_profile(profile)?;
        let store = Self::open(profile, from)?;
        if from == to {
            return Ok(());
        }
        store.transaction(|state| {
            state.grant_serial = state.grant_serial.max(
                state
                    .incoming
                    .iter()
                    .map(|grant| grant.version)
                    .max()
                    .unwrap_or(0),
            );
            state.origin = to.to_owned();
            state.incoming.clear();
            state.outgoing.clear();
            state.invitations.clear();
            state.pending.clear();
            Ok(())
        })
    }

    pub fn open(profile: &Path, origin: &str) -> Result<Self> {
        validate_origin(origin)?;
        let store = Self {
            profile: profile.to_owned(),
            identity: Identity::load(profile)?,
            origin: origin.to_owned(),
        };
        // Fail on an existing profile bound to a different identity or relay.
        store.read_state()?;
        Ok(store)
    }
    pub fn invite(&self, now: u64) -> Result<Invitation> {
        self.transaction(|state| {
            state.invitations.retain(|i| i.retain_until > now);
            if state.invitations.len() >= 128
                || state
                    .invitations
                    .iter()
                    .filter(|i| i.result.is_none() && i.expires_at > now)
                    .count()
                    >= 4
            {
                return Err(Error::new("QUOTA_EXCEEDED", "too many pairing invitations"));
            }
            let invitation = Invitation {
                version: 1,
                origin: self.origin.clone(),
                publisher: self.identity.id.clone(),
                certificate: hex::encode(&self.identity.certificate),
                invitation_id: random_id()?,
                secret: format!("{}{}", random_id()?, random_id()?),
                expires_at: now.saturating_add(600),
            };
            state.invitations.push(SavedInvitation {
                id: invitation.invitation_id.clone(),
                secret_hash: secret_hash(&invitation.secret),
                expires_at: invitation.expires_at,
                result: None,
                retain_until: invitation.expires_at,
            });
            Ok(invitation)
        })
    }
    pub fn prepare(&self, invitation: &Invitation, now: u64) -> Result<RedeemRequest> {
        invitation.validate()?;
        if invitation.origin != self.origin || invitation.publisher == self.identity.id {
            return Err(invalid());
        }
        self.transaction(|state| {
            state.pending.retain(|p| p.retain_until > now);
            let request_id = if let Some(p) = state.pending.iter().find(|p| {
                p.invitation_id == invitation.invitation_id && p.publisher == invitation.publisher
            }) {
                p.request_id.clone()
            } else {
                if invitation.expires_at <= now {
                    return Err(invalid());
                }
                if state.pending.len() >= 64 {
                    return Err(Error::new(
                        "QUOTA_EXCEEDED",
                        "too many pending pairing requests",
                    ));
                }
                let request_id = random_id()?;
                state.pending.push(Pending {
                    invitation_id: invitation.invitation_id.clone(),
                    publisher: invitation.publisher.clone(),
                    request_id: request_id.clone(),
                    retain_until: now.saturating_add(RETENTION),
                });
                request_id
            };
            Ok(RedeemRequest {
                invitation_id: invitation.invitation_id.clone(),
                secret: invitation.secret.clone(),
                request_id,
            })
        })
    }
    /// Call only with the peer certificate obtained from the authenticated TLS session.
    pub fn redeem(
        &self,
        peer: &CertificateDer<'_>,
        request: &RedeemRequest,
        now: u64,
    ) -> Result<Receipt> {
        let peer_id = fingerprint(peer)?;
        if peer_id == self.identity.id
            || !hex_id(&request.invitation_id, 32)
            || !hex_id(&request.request_id, 32)
            || !hex_id(&request.secret, 64)
        {
            return Err(invalid());
        }
        self.transaction(|state| {
            let invitation = state
                .invitations
                .iter_mut()
                .find(|i| i.id == request.invitation_id)
                .ok_or_else(invalid)?;
            if invitation.secret_hash != secret_hash(&request.secret)
                || invitation.retain_until <= now
            {
                return Err(invalid());
            }
            if let Some(result) = &invitation.result {
                return if result.connector == peer_id
                    && result.request_id == request.request_id
                    && state
                        .incoming
                        .iter()
                        .any(|g| g.device_id == peer_id && g.version == result.grant_version)
                {
                    Ok(result.clone())
                } else {
                    Err(invalid())
                };
            }
            if invitation.expires_at <= now {
                return Err(invalid());
            }
            let version =
                if let Some(grant) = state.incoming.iter().find(|g| g.device_id == peer_id) {
                    grant.version
                } else {
                    if state.incoming.len() >= 64 {
                        return Err(Error::new(
                            "QUOTA_EXCEEDED",
                            "authorized device limit reached",
                        ));
                    }
                    state.grant_serial = next_grant_version(state.grant_serial, &state.incoming)?;
                    state.incoming.push(Grant {
                        device_id: peer_id.clone(),
                        certificate: hex::encode(peer),
                        version: state.grant_serial,
                    });
                    state.grant_serial
                };
            let result = Receipt {
                publisher: self.identity.id.clone(),
                connector: peer_id,
                origin: self.origin.clone(),
                invitation_id: request.invitation_id.clone(),
                request_id: request.request_id.clone(),
                grant_version: version,
            };
            invitation.result = Some(result.clone());
            invitation.retain_until = now.saturating_add(RETENTION);
            Ok(result)
        })
    }
    pub fn complete(
        &self,
        invitation: &Invitation,
        request: &RedeemRequest,
        receipt: &Receipt,
        now: u64,
    ) -> Result<()> {
        invitation.validate()?;
        if receipt.publisher != invitation.publisher
            || receipt.connector != self.identity.id
            || receipt.origin != self.origin
            || receipt.origin != invitation.origin
            || receipt.invitation_id != invitation.invitation_id
            || receipt.invitation_id != request.invitation_id
            || receipt.request_id != request.request_id
            || receipt.grant_version == 0
        {
            return Err(invalid());
        }
        self.transaction(|state| {
            if !state.pending.iter().any(|p| {
                p.invitation_id == receipt.invitation_id
                    && p.publisher == receipt.publisher
                    && p.request_id == receipt.request_id
                    && p.retain_until > now
            }) {
                return Err(invalid());
            }
            let grant = Grant {
                device_id: receipt.publisher.clone(),
                certificate: invitation.certificate.clone(),
                version: receipt.grant_version,
            };
            if let Some(existing) = state
                .outgoing
                .iter_mut()
                .find(|g| g.device_id == receipt.publisher)
            {
                if existing.version > grant.version {
                    return Err(invalid());
                }
                *existing = grant;
            } else {
                if state.outgoing.len() >= 64 {
                    return Err(Error::new(
                        "QUOTA_EXCEEDED",
                        "accessible host limit reached",
                    ));
                }
                state.outgoing.push(grant);
            }
            Ok(())
        })
    }
    pub fn revoke_peer(&self, device: &str) -> Result<u64> {
        self.transaction(|state| {
            if !state.incoming.iter().any(|g| g.device_id == device) {
                return Err(Error::new(
                    "NOT_AUTHORIZED",
                    "this device has no incoming grant",
                ));
            }
            state.grant_serial = next_grant_version(state.grant_serial, &state.incoming)?;
            state.incoming.retain(|g| g.device_id != device);
            Ok(state.grant_serial)
        })
    }
    pub fn grants(&self) -> Result<Grants> {
        let state = self.read_state()?;
        Ok(Grants {
            allowed_here: state.incoming,
            accessible_hosts: state.outgoing,
        })
    }
    pub fn revoke_invitation(&self, id: &str) -> Result<()> {
        self.transaction(|state| {
            let invitation = state
                .invitations
                .iter()
                .find(|i| i.id == id && i.result.is_none())
                .ok_or_else(invalid)?;
            let id = invitation.id.clone();
            state.invitations.retain(|i| i.id != id);
            Ok(())
        })
    }
    fn read_state(&self) -> Result<State> {
        private_directory(&self.profile, false)?;
        let path = self.profile.join("pairing.json");
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(State {
                    schema: 1,
                    device_id: self.identity.id.clone(),
                    origin: self.origin.clone(),
                    invitations: vec![],
                    incoming: vec![],
                    outgoing: vec![],
                    pending: vec![],
                    grant_serial: 0,
                });
            }
            Err(e) => return Err(storage(e)),
        };
        private_file(&file)?;
        let mut bytes = Vec::new();
        file.take((STATE_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        if bytes.len() > STATE_LIMIT {
            return Err(Error::new(
                "PAIR_STORAGE",
                "pairing state exceeds size limit",
            ));
        }
        let state: State = serde_json::from_slice(&bytes).map_err(|_| {
            Error::new(
                "PAIR_STORAGE",
                "pairing state is corrupt; it was not replaced",
            )
        })?;
        if state.schema != 1 || state.device_id != self.identity.id || state.origin != self.origin {
            return Err(Error::new(
                "PROFILE_MISMATCH",
                "pairing state belongs to another identity, relay origin or schema",
            ));
        }
        Ok(state)
    }
    fn transaction<T>(&self, change: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        private_directory(&self.profile, false)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.profile.join("pairing.lock"))
            .map_err(storage)?;
        private_file(&lock)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(storage(std::io::Error::last_os_error()));
        }
        let mut state = self.read_state()?;
        let result = change(&mut state)?;
        let encoded = serde_json::to_vec(&state)
            .map_err(|_| Error::new("PAIR_STORAGE", "cannot encode pairing state"))?;
        if encoded.len() > STATE_LIMIT {
            return Err(Error::new(
                "QUOTA_EXCEEDED",
                "pairing state size limit reached",
            ));
        }
        let mut stage = tempfile::Builder::new()
            .prefix(".pairing-")
            .permissions(std::fs::Permissions::from_mode(0o600))
            .tempfile_in(&self.profile)
            .map_err(storage)?;
        stage
            .write_all(&encoded)
            .and_then(|_| stage.as_file().sync_all())
            .map_err(storage)?;
        stage
            .persist(self.profile.join("pairing.json"))
            .map_err(|e| storage(e.error))?;
        File::open(&self.profile)
            .and_then(|f| f.sync_all())
            .map_err(storage)?;
        Ok(result)
    }
}
fn next_grant_version(serial: u64, grants: &[Grant]) -> Result<u64> {
    let current = serial.max(grants.iter().map(|g| g.version).max().unwrap_or(0));
    if current >= 9_007_199_254_740_991 {
        return Err(Error::new(
            "QUOTA_EXCEEDED",
            "grant version space exhausted",
        ));
    }
    Ok(current + 1)
}

fn private_file(file: &File) -> Result<()> {
    let meta = file.metadata().map_err(storage)?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.len() > STATE_LIMIT as u64
    {
        return Err(Error::new(
            "PAIR_STORAGE",
            "pairing files must be private, owned regular files within the size limit",
        ));
    }
    Ok(())
}
pub fn fingerprint(certificate: &[u8]) -> Result<String> {
    Ok(hex::encode(Sha256::digest(certificate_spki(certificate)?)))
}
pub fn validate_origin(origin: &str) -> Result<()> {
    if origin.len() > 512 {
        return Err(invalid());
    }
    let parsed = url::Url::parse(origin).map_err(|_| invalid())?;
    if parsed.scheme() != "https"
        || parsed.origin().ascii_serialization() != origin
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(invalid());
    }
    Ok(())
}
fn hex_id(value: &str, size: usize) -> bool {
    value.len() == size
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn secret_hash(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}
fn invalid() -> Error {
    Error::new(
        "PAIR_INVALID",
        "invalid, expired, revoked, or already used pairing invitation",
    )
}
fn storage(error: std::io::Error) -> Error {
    Error::new("PAIR_STORAGE", error.to_string())
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
enum PairResponse {
    Granted { receipt: Receipt },
    Rejected,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairAck {
    request_id: String,
}

pub async fn serve<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: S,
    host: &Store,
    channel: &str,
    boot: &str,
) -> Result<()> {
    serve_after_commit(stream, host, channel, boot, || async { Ok(()) }).await
}

pub(crate) async fn serve_after_commit<S, F, Fut>(
    stream: S,
    host: &Store,
    channel: &str,
    boot: &str,
    after_commit: F,
) -> Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let (mut stream, peer) =
        crate::secure::accept_pair(stream, &host.identity, &host.origin, channel, boot).await?;
    let request: RedeemRequest = crate::control::receive(&mut stream).await?;
    let result = host.redeem(&peer, &request, now());
    if result.is_ok() {
        after_commit().await?;
    }
    // Never reveal invitation existence or storage details to a peer.
    let response = match &result {
        Ok(receipt) => PairResponse::Granted {
            receipt: receipt.clone(),
        },
        Err(_) => PairResponse::Rejected,
    };
    crate::control::send(&mut stream, &response).await?;
    // Closing the relay channel immediately can discard an in-flight response.
    // The connector confirms durable receipt before either endpoint closes.
    let ack: PairAck = crate::control::receive(&mut stream).await?;
    if ack.request_id != request.request_id {
        return Err(invalid());
    }
    use tokio::io::AsyncWriteExt;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), stream.shutdown()).await;
    result.map(|_| ())
}

pub async fn accept<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    stream: S,
    connector: &Store,
    invitation: &Invitation,
    channel: &str,
    boot: &str,
) -> Result<Receipt> {
    // Persist request ID before sending any redeem attempt; re-importing the
    // invitation recovers the same request after a lost response or restart.
    let request = connector.prepare(invitation, now())?;
    let context = crate::control::Context {
        version: 1,
        origin: invitation.origin.clone(),
        publisher: invitation.publisher.clone(),
        connector: connector.identity.id.clone(),
        purpose: crate::control::Purpose::Pair,
        grant_version: None,
        target: None,
        channel_id: channel.into(),
        boot_id: boot.into(),
    };
    let mut stream = crate::secure::connect(
        stream,
        &connector.identity,
        &invitation.peer_certificate()?,
        &context,
    )
    .await?;
    crate::control::send(&mut stream, &request).await?;
    let result = match crate::control::receive(&mut stream).await? {
        PairResponse::Granted { receipt } => {
            connector.complete(invitation, &request, &receipt, now())?;
            Ok(receipt)
        }
        PairResponse::Rejected => Err(invalid()),
    };
    // A lost acknowledgement must not turn a received result into uncertainty.
    // Lost responses remain recoverable using the persisted request ID.
    let _ = crate::control::send(
        &mut stream,
        &PairAck {
            request_id: request.request_id,
        },
    )
    .await;
    use tokio::io::AsyncWriteExt;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), stream.shutdown()).await;
    result
}
