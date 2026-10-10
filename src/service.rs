use crate::{
    Error, Result, control,
    identity::Identity,
    native::NativeTarget,
    pairing::{self, Store},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};
use tokio::net::{UnixListener, UnixStream};

#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Status,
    PowerStatus,
    Invite,
    ReloadTargets,
    ReloadAuthorization,
    RevokeInvitation {
        invitation_id: String,
    },
    Pair {
        publisher: String,
        channel_id: String,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
enum Response {
    PowerStatus {
        status: crate::power::Status,
    },
    Status {
        origin: String,
        relay_online: bool,
        pid: u32,
    },
    Invitation {
        value: String,
    },
    Ready {
        boot_id: String,
    },
    Done,
    Rejected {
        code: String,
    },
}

pub(crate) fn lock_profile(profile: &Path) -> Result<std::fs::File> {
    crate::identity::private_directory(profile, false)?;
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(profile.join("daemon.lock"))
        .map_err(io)?;
    let meta = lock.metadata().map_err(io)?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err(Error::new(
            "UNSAFE_SERVICE",
            "daemon lock must be an owned private file",
        ));
    }
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(Error::new(
            "SERVICE_BUSY",
            "this profile already has a running service",
        ));
    }
    Ok(lock)
}

pub async fn run(profile: &Path, origin: &str, remote: Option<crate::relay::Relay>) -> Result<()> {
    let lock = lock_profile(profile)?;
    let identity = Identity::load(profile)?;
    let registry = std::sync::Arc::new(crate::targets::Registry::open(profile)?);
    Store::open(profile, origin)?;
    let path = profile.join("daemon.sock");
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == unsafe { libc::geteuid() } => {
            std::fs::remove_file(&path).map_err(io)?
        }
        Ok(_) => {
            return Err(Error::new(
                "UNSAFE_SERVICE",
                "refusing to replace a non-socket service path",
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io(error)),
    }
    let listener = UnixListener::bind(&path).map_err(io)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(io)?;
    let cleanup = SocketCleanup {
        path,
        inode: std::fs::metadata(profile.join("daemon.sock"))
            .map_err(io)?
            .ino(),
    };
    let boot = control::random_id()?;
    let mut interrupt =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()).map_err(io)?;
    let mut terminate =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).map_err(io)?;
    let mut hangup =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).map_err(io)?;
    let power = crate::power::Guard::start();
    let mut tasks = tokio::task::JoinSet::new();
    let mut publisher = tokio::task::JoinSet::new();
    let mut synchronization = tokio::task::JoinSet::new();
    let mut changed_targets = false;
    if let Some(relay) = remote.clone() {
        let registry = registry.clone();
        publisher.spawn(async move { relay.publish_forever(registry).await });
    }
    let mut monitor = tokio::time::interval(std::time::Duration::from_millis(500));
    monitor.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut outcome = Ok(());
    eprintln!("Service ready: {} ({})", identity.id, origin);
    loop {
        tokio::select! {
            _ = interrupt.recv() => break,
            _ = terminate.recv() => break,
            _ = hangup.recv() => break,
            _ = monitor.tick() => {
                match registry.reload().await {
                    Ok(true) => changed_targets = true,
                    Ok(false) => {},
                    Err(error) => { outcome = Err(error); break; },
                }
            },
            result = publisher.join_next(), if !publisher.is_empty() => {
                outcome = result.unwrap().map_err(|_| Error::new("SERVICE_IO", "relay publisher task failed"))?;
                break;
            },
            _ = tasks.join_next(), if !tasks.is_empty() => {},
            _ = synchronization.join_next(), if !synchronization.is_empty() => {},
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(io)?;
                if tasks.len() >= 8 || stream.peer_cred().map_err(io)?.uid() != unsafe { libc::geteuid() } { continue; }
                let (profile, origin, boot) = (profile.to_owned(), origin.to_owned(), boot.clone());
                let remote = remote.clone();
                let registry = registry.clone();
                let power = power.status();
                tasks.spawn(async move {
                    let result = tokio::time::timeout(std::time::Duration::from_secs(30), handle(stream, &profile, &origin, &boot, remote.as_ref(), &registry, power)).await;
                    if let Ok(Err(error)) = result { eprintln!("Service request failed: {}", error.code); }
                });
            }
        }
        if changed_targets && synchronization.is_empty() {
            changed_targets = false;
            if let Some(remote) = remote.clone() {
                let registry = registry.clone();
                synchronization.spawn(async move {
                    if let Err(error) = remote.sync_current_state(&registry).await {
                        eprintln!("Target update saved locally: {}", error.code);
                    }
                });
            }
        }
    }
    tasks.shutdown().await;
    synchronization.shutdown().await;
    publisher.shutdown().await;
    power.stop().await;
    drop(listener);
    drop(cleanup);
    drop(lock);
    outcome
}

async fn handle(
    mut stream: UnixStream,
    profile: &Path,
    origin: &str,
    boot: &str,
    remote: Option<&crate::relay::Relay>,
    registry: &crate::targets::Registry,
    power: crate::power::Status,
) -> Result<()> {
    let store = Store::open(profile, origin)?;
    match control::receive(&mut stream).await? {
        Request::PowerStatus => {
            control::send(&mut stream, &Response::PowerStatus { status: power }).await
        }
        Request::Status => {
            control::send(
                &mut stream,
                &Response::Status {
                    origin: origin.to_owned(),
                    relay_online: remote.is_some_and(crate::relay::Relay::is_online),
                    pid: std::process::id(),
                },
            )
            .await
        }
        Request::ReloadAuthorization => {
            if let Some(remote) = remote {
                remote.authorization_changed();
                if let Err(error) = remote.sync_current_state(registry).await {
                    return reject(&mut stream, error).await;
                }
            }
            control::send(&mut stream, &Response::Done).await
        }
        Request::ReloadTargets => {
            registry.reload().await?;
            if let Some(remote) = remote {
                remote.sync_current_state(registry).await?;
            }
            control::send(&mut stream, &Response::Done).await
        }
        Request::Invite => match store.invite(pairing::now()) {
            Ok(invitation) => {
                if let Some(remote) = remote
                    && let Err(error) = remote.register_invitation(&invitation).await
                {
                    let _ = store.revoke_invitation(&invitation.invitation_id);
                    return reject(&mut stream, error).await;
                }
                control::send(
                    &mut stream,
                    &Response::Invitation {
                        value: invitation.encode()?,
                    },
                )
                .await
            }
            Err(error) => reject(&mut stream, error).await,
        },
        Request::RevokeInvitation { invitation_id } => {
            match store.revoke_invitation(&invitation_id) {
                Ok(()) => control::send(&mut stream, &Response::Done).await,
                Err(error) => reject(&mut stream, error).await,
            }
        }
        Request::Pair {
            publisher: expected,
            channel_id,
        } => {
            if Identity::load(profile)?.id != expected
                || channel_id.len() != 32
                || !channel_id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return reject(
                    &mut stream,
                    Error::new("PAIR_INVALID", "invalid pairing route"),
                )
                .await;
            }
            control::send(
                &mut stream,
                &Response::Ready {
                    boot_id: boot.into(),
                },
            )
            .await?;
            // This branch terminates after one encrypted redemption. It cannot
            // dispatch admin requests, list targets, or connect a native socket.
            pairing::serve(stream, &store, &channel_id, boot).await
        }
    }
}
async fn reject(stream: &mut UnixStream, error: Error) -> Result<()> {
    control::send(
        stream,
        &Response::Rejected {
            code: error.code.into(),
        },
    )
    .await?;
    Err(error)
}

pub async fn status(profile: &Path) -> Result<(String, bool, u32)> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut stream = connect(&profile.join("daemon.sock")).await?;
        control::send(&mut stream, &Request::Status).await?;
        match control::receive(&mut stream).await? {
            Response::Status {
                origin,
                relay_online,
                pid,
            } => Ok((origin, relay_online, pid)),
            _ => Err(Error::new(
                "SERVICE_IO",
                "unexpected service status response",
            )),
        }
    })
    .await
    .map_err(|_| Error::new("SERVICE_IO", "service status timed out"))?
}

/// Kept separate from the original status response so old and new CLI/service
/// binaries can still perform lifecycle operations during an upgrade.
pub async fn power_status(profile: &Path) -> Result<crate::power::Status> {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut stream = connect(&profile.join("daemon.sock")).await?;
        control::send(&mut stream, &Request::PowerStatus).await?;
        match control::receive(&mut stream).await? {
            Response::PowerStatus { status } => Ok(status),
            _ => Err(Error::new(
                "SERVICE_IO",
                "unexpected sleep protection response",
            )),
        }
    })
    .await
    .map_err(|_| Error::new("SERVICE_IO", "sleep protection status timed out"))?
}

pub async fn power_summary(profile: &Path) -> String {
    match power_status(profile).await {
        Ok(status) => status.to_string(),
        Err(_) => {
            "Sleep protection: unknown. Update/restart the service to report its inhibitors.".into()
        }
    }
}

pub async fn reload_authorization(profile: &Path) -> Result<()> {
    let path = profile.join("daemon.sock");
    if !path.exists() {
        return Ok(());
    }
    let mut stream = match connect(&path).await {
        Ok(stream) => stream,
        Err(error) if error.code == "HOST_OFFLINE" => return Ok(()),
        Err(error) => return Err(error),
    };
    control::send(&mut stream, &Request::ReloadAuthorization).await?;
    match control::receive(&mut stream).await? {
        Response::Done => Ok(()),
        Response::Rejected { code } => Err(Error::new(
            "REVOKED_LOCALLY",
            format!("local revocation is committed; relay synchronization failed ({code})"),
        )),
        _ => Err(Error::new(
            "REVOKED_LOCALLY",
            "local revocation is committed; service did not confirm relay synchronization",
        )),
    }
}

pub async fn reload_targets(profile: &Path) -> Result<()> {
    let path = profile.join("daemon.sock");
    if !path.exists() {
        return Ok(());
    }
    let mut stream = match connect(&path).await {
        Ok(stream) => stream,
        Err(error) if error.code == "HOST_OFFLINE" => return Ok(()),
        Err(error) => return Err(error),
    };
    control::send(&mut stream, &Request::ReloadTargets).await?;
    match control::receive(&mut stream).await? {
        Response::Done => Ok(()),
        _ => Err(Error::new(
            "TARGET_RELOAD_FAILED",
            "configuration saved; service did not confirm applying it",
        )),
    }
}

pub async fn create_invitation(profile: &Path) -> Result<pairing::Invitation> {
    let mut stream = connect(&profile.join("daemon.sock")).await?;
    control::send(&mut stream, &Request::Invite).await?;
    match control::receive(&mut stream).await? {
        Response::Invitation { value } => pairing::Invitation::decode(&value),
        Response::Rejected { code } => Err(Error::new("PAIR_FAILED", code)),
        _ => Err(Error::new(
            "PROTOCOL_ERROR",
            "unexpected local service response",
        )),
    }
}
pub async fn revoke_invitation(profile: &Path, invitation_id: &str) -> Result<()> {
    let mut stream = connect(&profile.join("daemon.sock")).await?;
    control::send(
        &mut stream,
        &Request::RevokeInvitation {
            invitation_id: invitation_id.into(),
        },
    )
    .await?;
    match control::receive(&mut stream).await? {
        Response::Done => Ok(()),
        Response::Rejected { code } => Err(Error::new("PAIR_FAILED", code)),
        _ => Err(Error::new(
            "PROTOCOL_ERROR",
            "unexpected local service response",
        )),
    }
}
pub async fn accept_local(
    profile: &Path,
    socket: &Path,
    invitation: &pairing::Invitation,
) -> Result<pairing::Receipt> {
    let connector = Store::open(profile, &invitation.origin)?;
    let mut stream = connect(socket).await?;
    let channel = control::random_id()?;
    control::send(
        &mut stream,
        &Request::Pair {
            publisher: invitation.publisher.clone(),
            channel_id: channel.clone(),
        },
    )
    .await?;
    match control::receive(&mut stream).await? {
        Response::Ready { boot_id } => {
            pairing::accept(stream, &connector, invitation, &channel, &boot_id).await
        }
        Response::Rejected { code } => Err(Error::new("PAIR_FAILED", code)),
        _ => Err(Error::new(
            "PROTOCOL_ERROR",
            "unexpected local service response",
        )),
    }
}
async fn connect(path: &Path) -> Result<UnixStream> {
    let target = NativeTarget::socket(path).map_err(|e| {
        if e.code == "TARGET_OFFLINE" {
            Error::new("HOST_OFFLINE", "start the publisher service before pairing")
        } else {
            e
        }
    })?;
    tokio::time::timeout(std::time::Duration::from_secs(3), target.connect())
        .await
        .map_err(|_| Error::new("HOST_OFFLINE", "local service connection timed out"))?
        .map_err(|e| {
            if e.code == "TARGET_OFFLINE" {
                Error::new("HOST_OFFLINE", "publisher service is offline")
            } else {
                e
            }
        })
}
struct SocketCleanup {
    path: PathBuf,
    inode: u64,
}
impl Drop for SocketCleanup {
    fn drop(&mut self) {
        if std::fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.file_type().is_socket() && m.ino() == self.inode)
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn io(error: std::io::Error) -> Error {
    Error::new("SERVICE_IO", error.to_string())
}
