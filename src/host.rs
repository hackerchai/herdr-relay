use crate::{
    Error, Result,
    control::{self, Context, Purpose},
    native::NativeTarget,
    pairing::{Grant, Store},
    secure,
};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};
use tokio::io::{AsyncRead, AsyncWrite};

pub struct SharedTarget {
    pub id: String,
    name: String,
    runtime_id: String,
    native: Option<NativeTarget>,
    available: std::sync::atomic::AtomicBool,
    pub(crate) definition: Option<crate::targets::Definition>,
    cancelled: tokio::sync::watch::Sender<bool>,
    entering: std::sync::Mutex<()>,
    active: tokio::sync::watch::Sender<usize>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetStatus {
    pub id: String,
    pub name: String,
    pub runtime_id: String,
    pub status: String,
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Response {
    Targets { targets: Vec<TargetStatus> },
    Ready { runtime_id: String },
    Started,
    Rejected { code: String },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Start {
    pub runtime_id: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ack {}
impl SharedTarget {
    pub fn new(name: String, native: NativeTarget) -> Result<Self> {
        if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(Error::new(
                "INVALID_TARGET",
                "target name must contain 1-128 bytes without control characters",
            ));
        }
        Ok(Self {
            id: control::random_id()?,
            name,
            runtime_id: control::random_id()?,
            native: Some(native),
            available: std::sync::atomic::AtomicBool::new(true),
            definition: None,
            cancelled: tokio::sync::watch::channel(false).0,
            entering: std::sync::Mutex::new(()),
            active: tokio::sync::watch::channel(0).0,
        })
    }
    pub(crate) fn from_definition(definition: crate::targets::Definition) -> Result<Self> {
        Ok(Self {
            id: definition.id.clone(),
            name: definition.name.clone(),
            runtime_id: control::random_id()?,
            native: definition.locator.resolve().ok(),
            available: std::sync::atomic::AtomicBool::new(true),
            definition: Some(definition),
            cancelled: tokio::sync::watch::channel(false).0,
            entering: std::sync::Mutex::new(()),
            active: tokio::sync::watch::channel(0).0,
        })
    }
    pub(crate) fn runtime_is_current(&self) -> bool {
        let current = self
            .definition
            .as_ref()
            .and_then(|d| d.locator.resolve().ok());
        match (&self.native, current) {
            (Some(old), Some(new)) => old.same_generation(&new),
            (None, None) => true,
            _ => false,
        }
    }
    fn enter(&self) -> Result<Active<'_>> {
        let _guard = self
            .entering
            .lock()
            .expect("target operation lock poisoned");
        if *self.cancelled.borrow() {
            return Err(Error::new("TARGET_DISABLED", "target generation ended"));
        }
        self.active.send_modify(|n| *n += 1);
        Ok(Active(self))
    }
    pub(crate) fn stop(&self) {
        let _guard = self
            .entering
            .lock()
            .expect("target operation lock poisoned");
        self.cancelled.send_replace(true);
    }
    pub(crate) async fn drained(&self) {
        let mut active = self.active.subscribe();
        while *active.borrow_and_update() != 0 {
            if active.changed().await.is_err() {
                break;
            }
        }
    }
    pub fn status(&self) -> TargetStatus {
        let status = match self
            .native
            .as_ref()
            .filter(|_| self.available.load(std::sync::atomic::Ordering::Relaxed))
            .ok_or_else(|| Error::new("TARGET_OFFLINE", "target is offline"))
            .and_then(|native| native.check_current())
            .and_then(|()| self.native.as_ref().unwrap().reserve())
        {
            Ok(_) => "online",
            Err(error) if error.code == "TARGET_BUSY" => "busy",
            Err(_) => "offline",
        };
        TargetStatus {
            id: self.id.clone(),
            name: self.name.clone(),
            runtime_id: self.runtime_id.clone(),
            status: status.into(),
        }
    }
}
struct Active<'a>(&'a SharedTarget);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.send_modify(|n| *n -= 1);
    }
}

pub fn grant(profile: &Path, origin: &str, device: &str, incoming: bool) -> Result<Grant> {
    let grants = Store::open(profile, origin)?.grants()?;
    let grants = if incoming {
        grants.allowed_here
    } else {
        grants.accessible_hosts
    };
    grants
        .into_iter()
        .find(|g| g.device_id == device)
        .ok_or_else(|| {
            Error::new(
                "NOT_AUTHORIZED",
                "no current device grant for this direction",
            )
        })
}
pub async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    profile: &Path,
    context: &Context,
    targets: &[std::sync::Arc<SharedTarget>],
) -> Result<()> {
    serve_after_auth(stream, profile, context, targets, || async { Ok(()) }).await
}

/// Serve one authorized channel. `authenticated` reports successful mutual
/// authentication to the relay; the native stream starts only after it succeeds.
pub async fn serve_after_auth<S, F, Fut>(
    stream: S,
    profile: &Path,
    context: &Context,
    targets: &[std::sync::Arc<SharedTarget>],
    authenticated: F,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let target = targets
        .iter()
        .find(|t| Some(&t.id) == context.target.as_ref());
    let _active = target.map(|t| t.enter()).transpose()?;
    let operation = serve_inner(stream, profile, context, targets, authenticated);
    if let Some(target) = target {
        let mut cancelled = target.cancelled.subscribe();
        if *cancelled.borrow() {
            return Err(Error::new("TARGET_DISABLED", "target generation ended"));
        }
        tokio::select! {
            biased;
            _ = cancelled.changed() => Err(Error::new("TARGET_DISABLED","target generation ended")),
            result = operation => result,
        }
    } else {
        operation.await
    }
}

async fn serve_inner<S, F, Fut>(
    stream: S,
    profile: &Path,
    context: &Context,
    targets: &[std::sync::Arc<SharedTarget>],
    authenticated: F,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let identity = crate::identity::Identity::load(profile)?;
    let saved = grant(profile, &context.origin, &context.connector, true)?;
    if context.grant_version != Some(saved.version) {
        return Err(Error::new("NOT_AUTHORIZED", "grant version changed"));
    }
    let certificate = hex::decode(&saved.certificate)
        .map_err(|_| Error::new("INVALID_CERTIFICATE", "invalid saved grant certificate"))?
        .into();
    let mut stream = secure::accept(stream, &identity, &certificate, context).await?;
    let target = targets
        .iter()
        .find(|t| Some(&t.id) == context.target.as_ref());
    let mut started = false;
    let operation = async {
        if grant(profile, &context.origin, &context.connector, true)?.version != saved.version {
            return Err(Error::new("NOT_AUTHORIZED", "device grant changed"));
        }
        // The relay acknowledgement takes its own round trips; overlap it with
        // the endpoint exchange. Native bytes still wait for both to succeed.
        let authenticated = authenticated();
        match context.purpose {
            Purpose::Control => {
                let exchange = async {
                    control::send(
                        &mut stream,
                        &Response::Targets {
                            targets: targets.iter().map(|t| t.status()).collect(),
                        },
                    )
                    .await?;
                    let _: Ack = control::receive(&mut stream).await?;
                    Ok(())
                };
                tokio::try_join!(exchange, authenticated).map(|_| ())
            }
            Purpose::Attach => {
                let target = target
                    .filter(|t| Some(&t.id) == context.target.as_ref())
                    .ok_or_else(|| Error::new("TARGET_NOT_SHARED", "target is not shared"))?;
                let native_target = target
                    .native
                    .as_ref()
                    .ok_or_else(|| Error::new("TARGET_OFFLINE", "target is offline"))?;
                native_target.check_current()?;
                let reservation = native_target.reserve()?;
                let exchange = async {
                    control::send(
                        &mut stream,
                        &Response::Ready {
                            runtime_id: target.runtime_id.clone(),
                        },
                    )
                    .await?;
                    let start: Start = control::receive(&mut stream).await?;
                    if start.runtime_id != target.runtime_id {
                        return Err(Error::new("TARGET_CHANGED", "target runtime changed"));
                    }
                    Ok(())
                };
                tokio::try_join!(exchange, authenticated)?;
                if grant(profile, &context.origin, &context.connector, true)?.version
                    != saved.version
                {
                    return Err(Error::new("NOT_AUTHORIZED", "device grant changed"));
                }
                let native = tokio::time::timeout(Duration::from_secs(4), native_target.connect())
                    .await
                    .map_err(|_| Error::new("TARGET_OFFLINE", "native socket timed out"))
                    .and_then(|result| result);
                target
                    .available
                    .store(native.is_ok(), std::sync::atomic::Ordering::Relaxed);
                let native = native?;
                started = true;
                control::send(&mut stream, &Response::Started).await?;
                // No further control messages are written after this boundary.
                let result =
                    crate::transport::bridge_native(&mut stream, native, Duration::from_secs(4))
                        .await;
                // Herdr may close just this native socket when its client
                // detaches. EOF does not establish that the shared server is
                // offline. Socket generation checks and failed future dials
                // determine server availability instead.
                drop(reservation);
                result.map(|_| ())
            }
            Purpose::Pair => Err(Error::new(
                "PROTOCOL_ERROR",
                "pairing has a separate entry point",
            )),
        }
    };
    let operation = operation.await;
    // Once Started is sent the stream is native bytes only, even on errors.
    // Bridge failures are handled by closure rather than injecting a response.
    if let Err(error) = &operation
        && !started
        && error.code != "TARGET_DISABLED"
        && control::send(
            &mut stream,
            &Response::Rejected {
                code: error.code.into(),
            },
        )
        .await
        .is_ok()
    {
        let _: Result<Ack> = control::receive(&mut stream).await;
    }
    operation
}
