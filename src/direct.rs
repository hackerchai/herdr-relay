//! Direct LAN/tailnet path. A host listens on TCP and advertises its private
//! addresses inside the authenticated control channel. A client on the same
//! network connects directly with the same pinned mutual TLS and context
//! binding used through the relay; any failure falls back to the relay.
use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

/// At most this many advertised addresses are kept or tried.
pub const MAX_ADDRESSES: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    /// Changes whenever the host's listener restarts; bound as the channel
    /// generation so a stale advertisement cannot reach a new listener.
    pub listener_id: String,
    pub port: u16,
    pub addresses: Vec<IpAddr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalNet {
    pub ip: IpAddr,
    pub prefix: u8,
}

fn cgnat(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 100 && (64..128).contains(&b)
}
fn ula(ip: Ipv6Addr) -> bool {
    ip.segments()[0] & 0xfe00 == 0xfc00
}
/// Tailscale's address ranges; its IPv4 interfaces use /32 prefixes, so
/// reachability cannot be inferred from the subnet.
fn tailnet(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => cgnat(ip),
        IpAddr::V6(ip) => ip.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0],
    }
}

/// Private LAN and tailnet addresses; never loopback, link-local or public.
pub fn advertisable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || cgnat(ip),
        IpAddr::V6(ip) => ula(ip),
    }
}

/// Peers a host accepts on its direct listener.
pub fn local_peer(ip: IpAddr) -> bool {
    let ip = match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        ip => ip,
    };
    advertisable(ip)
        || ip.is_loopback()
        || match ip {
            IpAddr::V4(ip) => ip.is_link_local(),
            IpAddr::V6(ip) => ip.segments()[0] & 0xffc0 == 0xfe80,
        }
}

fn same_subnet(a: IpAddr, b: IpAddr, prefix: u8) -> bool {
    match (a, b) {
        (IpAddr::V4(a), IpAddr::V4(b)) if prefix <= 32 => {
            let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
            u32::from(a) & mask == u32::from(b) & mask
        }
        (IpAddr::V6(a), IpAddr::V6(b)) if prefix <= 128 => {
            let mask = u128::MAX.checked_shl(128 - u32::from(prefix)).unwrap_or(0);
            u128::from(a) & mask == u128::from(b) & mask
        }
        _ => false,
    }
}

/// Addresses worth trying from this client, best first: hosts on one of the
/// client's own subnets, then tailnet addresses when the client is on a tailnet.
pub fn candidates(endpoint: &Endpoint, local: &[LocalNet]) -> Vec<SocketAddr> {
    let advertised: Vec<IpAddr> = endpoint
        .addresses
        .iter()
        .copied()
        .filter(|ip| advertisable(*ip))
        .collect();
    let subnet = advertised.iter().filter(|ip| {
        local
            .iter()
            .any(|net| !tailnet(net.ip) && same_subnet(**ip, net.ip, net.prefix))
    });
    let on_tailnet = |v4: bool| local.iter().any(|n| tailnet(n.ip) && n.ip.is_ipv4() == v4);
    let tail = advertised
        .iter()
        .filter(|ip| tailnet(**ip) && on_tailnet(ip.is_ipv4()));
    let mut chosen: Vec<SocketAddr> = vec![];
    for ip in subnet.chain(tail) {
        let address = SocketAddr::new(*ip, endpoint.port);
        if !chosen.contains(&address) && chosen.len() < MAX_ADDRESSES {
            chosen.push(address);
        }
    }
    chosen
}

/// This machine's non-loopback interface networks.
pub fn local_networks() -> Vec<LocalNet> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback())
        .map(|i| match i.addr {
            if_addrs::IfAddr::V4(a) => LocalNet {
                ip: IpAddr::V4(a.ip),
                prefix: a.prefixlen,
            },
            if_addrs::IfAddr::V6(a) => LocalNet {
                ip: IpAddr::V6(a.ip),
                prefix: a.prefixlen,
            },
        })
        .collect()
}

/// This host's advertisable addresses.
pub fn advertised_addresses() -> Vec<IpAddr> {
    let mut addresses: Vec<IpAddr> = local_networks()
        .into_iter()
        .map(|n| n.ip)
        .filter(|ip| advertisable(*ip))
        .collect();
    addresses.dedup();
    addresses.truncate(MAX_ADDRESSES);
    addresses
}

use crate::{
    Error, Result,
    control::{self, Context, Purpose},
    identity::Identity,
    pairing::Store,
};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::net::{TcpListener, TcpStream};

/// Default TCP port of the direct listener.
pub const DEFAULT_PORT: u16 = 47120;
const CONNECT_TIMEOUT: Duration = Duration::from_millis(700);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(3);
/// Later candidates start only if earlier ones have not connected by then.
const ATTEMPT_DELAY: Duration = Duration::from_millis(250);

pub type DirectStream = tokio_rustls::client::TlsStream<Box<dyn crate::relay::Transport>>;
pub struct Connected {
    pub stream: DirectStream,
    pub address: SocketAddr,
}

/// Low-latency settings; keepalive notices a sleeping or vanished peer.
fn configure(stream: &TcpStream) {
    let _ = stream.set_nodelay(true);
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(15))
        .with_interval(Duration::from_secs(5))
        .with_retries(3);
    let _ = socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive);
}

pub struct Listener {
    listener: TcpListener,
    id: String,
}
impl Listener {
    pub async fn bind(address: SocketAddr) -> Result<Self> {
        let listener = TcpListener::bind(address)
            .await
            .map_err(|e| Error::new("DIRECT_UNAVAILABLE", e.to_string()))?;
        Ok(Self {
            listener,
            id: control::random_id()?,
        })
    }
    /// Bind the preferred port, or any free port when it is taken.
    pub async fn bind_port(port: u16) -> Result<Self> {
        let any = |port| SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), port);
        let v4 = |port| SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), port);
        for address in [any(port), v4(port), any(0), v4(0)] {
            if let Ok(listener) = Self::bind(address).await {
                return Ok(listener);
            }
        }
        Err(Error::new(
            "DIRECT_UNAVAILABLE",
            "no TCP port for direct connections",
        ))
    }
    pub fn endpoint(&self) -> Endpoint {
        Endpoint {
            listener_id: self.id.clone(),
            port: self.listener.local_addr().map_or(0, |a| a.port()),
            addresses: advertised_addresses(),
        }
    }
    /// Accept direct channels until the task is dropped.
    pub async fn serve(
        self,
        profile: &Path,
        origin: &str,
        registry: Arc<crate::targets::Registry>,
        revocations: tokio::sync::watch::Receiver<u64>,
    ) -> Result<()> {
        // Unauthenticated handshakes are bounded like relay handshakes.
        let pending = Arc::new(tokio::sync::Semaphore::new(8));
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                accepted = self.listener.accept() => {
                    let Ok((stream, peer)) = accepted else { continue };
                    if !local_peer(peer.ip()) || tasks.len() >= 32 { continue; }
                    let Ok(permit) = pending.clone().try_acquire_owned() else { continue };
                    let (profile, origin, id) = (profile.to_owned(), origin.to_owned(), self.id.clone());
                    let (registry, revocations) = (registry.clone(), revocations.clone());
                    tasks.spawn(crate::timing::scope("serve_direct", async move {
                        // Unauthenticated attempts (scans, abandoned races) end silently.
                        let result = accept(stream, &profile, &origin, &id, &registry, revocations, permit).await;
                        if let Ok(Err(error)) = result { eprintln!("Direct channel ended: {}", error.code); }
                    }));
                }
            }
        }
    }
}

async fn accept(
    stream: TcpStream,
    profile: &Path,
    origin: &str,
    listener_id: &str,
    registry: &crate::targets::Registry,
    mut revocations: tokio::sync::watch::Receiver<u64>,
    permit: tokio::sync::OwnedSemaphorePermit,
) -> Result<Result<()>> {
    configure(&stream);
    let identity = Identity::load(profile)?;
    let grants = Store::open(profile, origin)?.grants()?.allowed_here;
    let certificates = grants
        .iter()
        .map(|g| hex::decode(&g.certificate).map(Into::into))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| Error::new("INVALID_CERTIFICATE", "invalid saved grant certificate"))?;
    let (mut stream, certificate) =
        crate::secure::accept_granted(stream, &identity, &certificates).await?;
    let connector = crate::pairing::fingerprint(&certificate)?;
    let grant = grants
        .iter()
        .find(|g| g.device_id == connector)
        .ok_or_else(|| Error::new("NOT_AUTHORIZED", "device grant is no longer current"))?;
    let context = control::bind_accept(&mut stream, |c: &Context| {
        if c.origin != origin
            || c.publisher != identity.id
            || c.connector != connector
            || c.grant_version != Some(grant.version)
            || c.boot_id != listener_id
            || c.purpose == Purpose::Pair
        {
            return Err(Error::new(
                "CONTEXT_MISMATCH",
                "direct channel context does not match this host",
            ));
        }
        Ok(())
    })
    .await?;
    crate::timing::mark("bind");
    drop(permit);
    // Errors after authentication are reported; earlier ones are not.
    Ok(async {
        let allowed = || {
            crate::host::grant(profile, origin, &connector, true)
                .is_ok_and(|g| g.version == grant.version)
        };
        let targets = registry.snapshot();
        let operation =
            crate::host::serve_direct(stream, profile, &context, &targets, grant.version);
        tokio::pin!(operation);
        loop {
            tokio::select! {
                biased;
                _ = revocations.changed() => if !allowed() {
                    return Err(Error::new("NOT_AUTHORIZED", "device authorization ended"));
                },
                result = &mut operation => return result,
            }
        }
    }
    .await)
}

/// Connect to the first reachable address and authenticate the host with
/// the same pinned mutual TLS and context binding as a relayed channel.
pub async fn connect_to(
    addresses: &[SocketAddr],
    profile: &Path,
    origin: &str,
    publisher: &str,
    listener_id: &str,
    purpose: Purpose,
    target: Option<&str>,
) -> Result<Connected> {
    use futures_util::StreamExt;
    let identity = Identity::load(profile)?;
    let grant = crate::host::grant(profile, origin, publisher, false)?;
    let certificate = hex::decode(&grant.certificate)
        .map_err(|_| Error::new("INVALID_CERTIFICATE", "invalid saved grant certificate"))?
        .into();
    let context = Context {
        version: 1,
        origin: origin.to_owned(),
        publisher: publisher.to_owned(),
        connector: identity.id.clone(),
        purpose,
        grant_version: Some(grant.version),
        target: target.map(str::to_owned),
        channel_id: control::random_id()?,
        boot_id: listener_id.to_owned(),
    };
    context.validate()?;
    let unreachable = || Error::new("DIRECT_UNREACHABLE", "host is not reachable directly");
    let mut attempts: futures_util::stream::FuturesUnordered<_> = addresses
        .iter()
        .enumerate()
        .map(|(index, address)| async move {
            tokio::time::sleep(ATTEMPT_DELAY * index as u32).await;
            let stream = tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await;
            (*address, stream)
        })
        .collect();
    // A TCP answer may come from another service or device reusing the same
    // numbering; a failed handshake moves on to the remaining candidates.
    loop {
        let (address, stream) = match attempts.next().await {
            Some((address, Ok(Ok(stream)))) => (address, stream),
            Some(_) => continue,
            None => return Err(unreachable()),
        };
        crate::timing::mark("direct_tcp");
        configure(&stream);
        let stream: Box<dyn crate::relay::Transport> = Box::new(stream);
        if let Ok(Ok(stream)) = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            crate::secure::connect(stream, &identity, &certificate, &context),
        )
        .await
        {
            return Ok(Connected { stream, address });
        }
    }
}

/// Try the endpoint's addresses that are plausibly reachable from here.
pub async fn connect(
    endpoint: &Endpoint,
    profile: &Path,
    origin: &str,
    publisher: &str,
    purpose: Purpose,
    target: Option<&str>,
) -> Result<Connected> {
    let addresses = candidates(endpoint, &local_networks());
    if addresses.is_empty() {
        return Err(Error::new(
            "DIRECT_UNAVAILABLE",
            "no shared network with the host",
        ));
    }
    connect_to(
        &addresses,
        profile,
        origin,
        publisher,
        &endpoint.listener_id,
        purpose,
        target,
    )
    .await
}

/// This process's listener generation and port, set once it is bound.
static LOCAL: std::sync::RwLock<Option<(String, u16)>> = std::sync::RwLock::new(None);

/// Publish (or withdraw) this host's direct listener in host information.
pub fn advertise(listener: Option<(String, u16)>) {
    *LOCAL.write().expect("direct endpoint lock poisoned") = listener;
}
/// The endpoint to advertise now; interface addresses are read fresh.
pub fn local_endpoint() -> Option<Endpoint> {
    let (listener_id, port) = LOCAL
        .read()
        .expect("direct endpoint lock poisoned")
        .clone()?;
    let addresses = advertised_addresses();
    (!addresses.is_empty()).then_some(Endpoint {
        listener_id,
        port,
        addresses,
    })
}
impl Endpoint {
    pub fn valid(&self) -> bool {
        self.listener_id.len() == 32
            && self
                .listener_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && self.port != 0
            && !self.addresses.is_empty()
            && self.addresses.len() <= MAX_ADDRESSES
            && self.addresses.iter().all(|ip| advertisable(*ip))
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cached {
    origin: String,
    identity: String,
    endpoint: Endpoint,
    /// Unix time before which direct attempts are skipped after a failure.
    #[serde(default)]
    retry_after: u64,
}
/// After a failed direct attempt, use the relay alone for this long unless
/// the host advertises a different endpoint.
pub const RETRY_AFTER_FAILURE: u64 = 600;
const CACHE: &str = "direct-endpoints.json";

/// Serialize read-modify-write cycles of the cache across processes.
fn locked<T>(profile: &Path, update: impl FnOnce() -> Result<T>) -> Result<T> {
    use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
    crate::identity::private_directory(profile, false)?;
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(profile.join("direct-endpoints.lock"))
        .map_err(|e| Error::new("DIRECT_CACHE", e.to_string()))?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(Error::new(
            "DIRECT_CACHE",
            std::io::Error::last_os_error().to_string(),
        ));
    }
    update()
}

fn read_cache(profile: &Path) -> Vec<Cached> {
    use std::{
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(profile.join(CACHE))
    else {
        return vec![];
    };
    let Ok(meta) = file.metadata() else {
        return vec![];
    };
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return vec![];
    }
    let mut bytes = Vec::new();
    if file.take(65537).read_to_end(&mut bytes).is_err() || bytes.len() > 65536 {
        return vec![];
    }
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Cache the endpoint a host advertised inside its authenticated channel.
pub fn remember(profile: &Path, origin: &str, identity: &str, endpoint: &Endpoint) -> Result<()> {
    if !endpoint.valid() {
        return Err(Error::new("PROTOCOL_ERROR", "invalid direct endpoint"));
    }
    locked(profile, || {
        let mut records = read_cache(profile);
        if records
            .iter()
            .any(|r| r.origin == origin && r.identity == identity && &r.endpoint == endpoint)
        {
            return Ok(());
        }
        // A changed endpoint deserves a fresh attempt; an unchanged one keeps its back-off.
        let retry_after = records
            .iter()
            .find(|r| r.origin == origin && r.identity == identity && &r.endpoint == endpoint)
            .map_or(0, |r| r.retry_after);
        records.retain(|r| !(r.origin == origin && r.identity == identity));
        records.push(Cached {
            origin: origin.to_owned(),
            identity: identity.to_owned(),
            endpoint: endpoint.clone(),
            retry_after,
        });
        if records.len() > 256 {
            records.remove(0);
        }
        write_cache(profile, &records)
    })
}
/// Skip direct attempts to this host for a while after a failure.
pub fn back_off(profile: &Path, origin: &str, identity: &str) {
    let _ = locked(profile, || {
        let mut records = read_cache(profile);
        let Some(record) = records
            .iter_mut()
            .find(|r| r.origin == origin && r.identity == identity)
        else {
            return Ok(());
        };
        record.retry_after = crate::pairing::now() + RETRY_AFTER_FAILURE;
        write_cache(profile, &records)
    });
}
fn write_cache(profile: &Path, records: &[Cached]) -> Result<()> {
    use std::{io::Write, os::unix::fs::PermissionsExt};
    let storage = |e: std::io::Error| Error::new("DIRECT_CACHE", e.to_string());
    let bytes = serde_json::to_vec(records).map_err(|_| Error::new("DIRECT_CACHE", "encode"))?;
    let mut stage = tempfile::NamedTempFile::new_in(profile).map_err(storage)?;
    stage
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(storage)?;
    stage.write_all(&bytes).map_err(storage)?;
    stage
        .persist(profile.join(CACHE))
        .map_err(|e| storage(e.error))?;
    Ok(())
}
/// Drop a cached endpoint the host no longer advertises.
pub fn forget(profile: &Path, origin: &str, identity: &str) {
    if recall(profile, origin, identity).is_none() {
        return;
    }
    let _ = locked(profile, || {
        let mut records = read_cache(profile);
        records.retain(|r| !(r.origin == origin && r.identity == identity));
        write_cache(profile, &records)
    });
}
/// The cached endpoint, if any.
pub fn recall(profile: &Path, origin: &str, identity: &str) -> Option<Endpoint> {
    read_cache(profile)
        .into_iter()
        .find(|r| r.origin == origin && r.identity == identity)
        .map(|r| r.endpoint)
        .filter(Endpoint::valid)
}
/// The cached endpoint unless a recent failure suppresses direct attempts.
pub fn usable(profile: &Path, origin: &str, identity: &str) -> Option<Endpoint> {
    read_cache(profile)
        .into_iter()
        .find(|r| r.origin == origin && r.identity == identity)
        .filter(|r| r.retry_after <= crate::pairing::now())
        .map(|r| r.endpoint)
        .filter(Endpoint::valid)
}

/// How a channel reached the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    Direct(SocketAddr),
    Relay,
}
impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct(address) if tailnet(address.ip()) => {
                write!(f, "direct (tailnet {})", address.ip())
            }
            Self::Direct(address) => write!(f, "direct (LAN {})", address.ip()),
            Self::Relay => f.write_str("relay"),
        }
    }
}

/// Prefer a direct attempt; open the relay only when it is unavailable or
/// fails. Also returns the direct failure so the caller can back off.
pub async fn with_fallback<T, D, R, Fut>(
    direct: Option<D>,
    relay: R,
) -> (Result<T>, Route, Option<Error>)
where
    D: std::future::Future<Output = Result<(T, SocketAddr)>>,
    R: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let failure = match direct {
        Some(attempt) => match attempt.await {
            Ok((value, address)) => return (Ok(value), Route::Direct(address), None),
            Err(error) => Some(error),
        },
        None => None,
    };
    (relay().await, Route::Relay, failure)
}

/// The listener port from `HERDR_RELAY_DIRECT` and `HERDR_RELAY_DIRECT_PORT`
/// values; `None` disables direct connections.
pub fn listener_port(enabled: Option<&str>, port: Option<&str>) -> Option<u16> {
    if enabled.is_some_and(|v| {
        matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "off" | "0" | "false" | "no"
        )
    }) {
        return None;
    }
    Some(
        port.and_then(|p| p.trim().parse().ok())
            .unwrap_or(DEFAULT_PORT),
    )
}
/// The environment, else `config.toml`, else the default port.
pub fn configured_port() -> Option<u16> {
    let config = crate::config::current();
    let var = |name| std::env::var(name).ok().filter(|v: &String| !v.is_empty());
    listener_port(
        var("HERDR_RELAY_DIRECT")
            .or_else(|| {
                config
                    .direct
                    .map(|on| if on { "on" } else { "off" }.to_owned())
            })
            .as_deref(),
        var("HERDR_RELAY_DIRECT_PORT")
            .or_else(|| config.direct_port.map(|port| port.to_string()))
            .as_deref(),
    )
}
