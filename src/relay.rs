use crate::{
    Error, Result,
    access::{self, Challenge},
    control::{self, Context, Purpose, random_id},
    frame::{Frame, Kind},
    host::{self, TargetStatus},
    identity::Identity,
    pairing::{self, Invitation, Receipt, Store},
};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
pub trait Transport: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}
impl<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send> Transport for T {}
type ChannelStream = Box<dyn Transport>;
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Clone)]
pub struct Relay {
    profile: PathBuf,
    identity: Arc<Identity>,
    origin: String,
    base: String,
    http: reqwest::Client,
    state_sync: Arc<tokio::sync::Mutex<()>>,
    revocations: tokio::sync::watch::Sender<u64>,
    online_boot: tokio::sync::watch::Sender<Option<String>>,
    state_ready: Arc<std::sync::atomic::AtomicBool>,
    pending: Arc<tokio::sync::Semaphore>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Opened {
    channel_id: String,
    boot_id: String,
    ticket: String,
    #[serde(default)]
    grant_version: Option<u64>,
    #[serde(default)]
    limits: Option<crate::stream::Limits>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum Event {
    Online {
        boot_id: String,
    },
    Pair {
        channel_id: String,
        connector: String,
        boot_id: String,
        ticket: String,
    },
    Access {
        channel_id: String,
        connector: String,
        boot_id: String,
        ticket: String,
        purpose: Purpose,
        target_id: Option<String>,
        grant_version: u64,
        #[serde(default)]
        limits: Option<crate::stream::Limits>,
    },
    RouteReady {
        channel_id: String,
        boot_id: String,
    },
}

impl Relay {
    pub async fn operator_status(&self) -> Result<serde_json::Value> {
        self.post("/v1/operator/status", &serde_json::json!({}))
            .await
    }
    pub fn is_online(&self) -> bool {
        self.state_ready.load(std::sync::atomic::Ordering::Acquire)
    }
    pub fn new(profile: &Path, origin: &str, local_url: Option<&str>) -> Result<Self> {
        pairing::validate_origin(origin)?;
        let base = if let Some(local) = local_url {
            let u = url::Url::parse(local).map_err(|_| protocol())?;
            if u.scheme() != "http"
                || !matches!(u.host_str(), Some("127.0.0.1" | "[::1]"))
                || u.origin().ascii_serialization() != local
                || !u.username().is_empty()
                || u.password().is_some()
            {
                return Err(Error::new(
                    "INVALID_RELAY",
                    "development transport must be an explicit HTTP loopback origin",
                ));
            }
            local.to_owned()
        } else {
            origin.to_owned()
        };
        let _ = rustls::crypto::ring::default_provider().install_default();
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .build()
            .map_err(network)?;
        Ok(Self {
            profile: profile.to_owned(),
            identity: Arc::new(Identity::load(profile)?),
            origin: origin.to_owned(),
            base,
            http,
            state_sync: Arc::new(tokio::sync::Mutex::new(())),
            revocations: tokio::sync::watch::channel(0).0,
            online_boot: tokio::sync::watch::channel(None).0,
            state_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            pending: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }
    pub(crate) fn check_identity(&self) -> Result<()> {
        if Identity::load(&self.profile)?.id != self.identity.id {
            return Err(Error::new(
                "IDENTITY_CHANGED",
                "device identity changed; restart and pair again",
            ));
        }
        Ok(())
    }
    pub(crate) async fn sync_current_state(
        &self,
        registry: &crate::targets::Registry,
    ) -> Result<()> {
        let boot = self.online_boot.borrow().clone();
        // Offline changes are already persisted locally. Every reconnect
        // mirrors the newest local state before reporting online.
        if let Some(boot) = boot {
            self.sync_state(&boot, registry).await?;
        }
        Ok(())
    }
    pub async fn publish_forever(&self, registry: Arc<crate::targets::Registry>) -> Result<()> {
        let mut backoff = crate::retry::Backoff::default();
        loop {
            self.check_identity()?;
            let boot = random_id()?;
            let began = tokio::time::Instant::now();
            let error = self
                .publish(&boot, registry.clone())
                .await
                .err()
                .unwrap_or_else(disconnected);
            self.state_ready
                .store(false, std::sync::atomic::Ordering::Release);
            self.online_boot.send_replace(None);
            if began.elapsed() >= Duration::from_secs(60) {
                backoff.reset();
            }
            let Some(delay) = backoff.next(&error, crate::retry::jitter()?) else {
                return Err(error);
            };
            eprintln!(
                "Relay offline ({}); retrying in {:.1}s",
                error.code,
                delay.as_secs_f64()
            );
            crate::retry::wait(delay).await;
        }
    }
    async fn proof(&self, method: &str, path: &str, payload: &str) -> Result<String> {
        let public = access::public_key(&self.identity)?;
        let hash = hex::encode(Sha256::digest(payload.as_bytes()));
        let request = serde_json::json!({"version":1,"origin":self.origin,"method":method,"path":path,"device_id":self.identity.id,"public_key":public,"payload_sha256":hash});
        let response = self
            .http
            .post(format!("{}/v1/challenge", self.base))
            .json(&request)
            .send()
            .await
            .map_err(network)?;
        let challenge: Challenge = response_json(response).await?;
        if challenge.origin != self.origin
            || challenge.method != method
            || challenge.path != path
            || challenge.public_key != public
            || challenge.payload_sha256 != hash
        {
            return Err(Error::new(
                "ACCESS_INVALID",
                "relay returned a challenge for a different operation",
            ));
        }
        serde_json::to_string(&challenge.sign(&self.identity, pairing::now())?)
            .map_err(|_| protocol())
    }
    async fn post<T: Serialize, R: DeserializeOwned>(&self, path: &str, payload: &T) -> Result<R> {
        let body = serde_json::to_string(payload).map_err(|_| protocol())?;
        if body.len() > 8192 {
            return Err(protocol());
        }
        let proof = self.proof("POST", path, &body).await?;
        let response = self
            .http
            .post(format!("{}{}", self.base, path))
            .header("X-Relay-Proof", proof)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(network)?;
        response_json(response).await
    }
    async fn websocket(&self, path: &str, header: &str, value: &str) -> Result<Socket> {
        let proof = self.proof("GET", path, value).await?;
        let mut url = url::Url::parse(&format!("{}{}", self.base, path)).map_err(|_| protocol())?;
        url.set_scheme(if self.base.starts_with("https:") {
            "wss"
        } else {
            "ws"
        })
        .map_err(|_| protocol())?;
        let mut request = url.as_str().into_client_request().map_err(|_| protocol())?;
        request
            .headers_mut()
            .insert("X-Relay-Proof", proof.parse().map_err(|_| protocol())?);
        request.headers_mut().insert(
            tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(header.as_bytes())
                .map_err(|_| protocol())?,
            value.parse().map_err(|_| protocol())?,
        );
        let config = WebSocketConfig::default()
            .read_buffer_size(16 * 1024)
            .write_buffer_size(16 * 1024)
            .max_write_buffer_size(64 * 1024)
            .max_frame_size(Some(16416))
            .max_message_size(Some(16416));
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async_with_config(request, Some(config), true),
        )
        .await
        .map_err(|_| Error::new("NETWORK_LOST", "relay WebSocket connection timed out"))?;
        result.map(|(socket, _)| socket).map_err(socket_error)
    }
    pub async fn register_invitation(&self, invitation: &Invitation) -> Result<()> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Registered {
            registered: bool,
        }
        let response: Registered = self.post(&format!("/v1/devices/{}/invitations",self.identity.id), &serde_json::json!({"invitation_id":invitation.invitation_id,"expires_at":invitation.expires_at})).await?;
        if !response.registered {
            return Err(protocol());
        }
        Ok(())
    }
    pub async fn accept(&self, invitation: &Invitation) -> Result<Receipt> {
        let store = Store::open(&self.profile, &self.origin)?;
        let channel = random_id()?;
        let opened: Opened = self
            .post(
                &format!("/v1/devices/{}/pair", invitation.publisher),
                &serde_json::json!({"invitation_id":invitation.invitation_id,"channel_id":channel}),
            )
            .await?;
        if opened.channel_id != channel {
            return Err(protocol());
        }
        let stream = self
            .join(
                &invitation.publisher,
                &channel,
                "connector",
                &opened.ticket,
                &opened.boot_id,
                None,
            )
            .await?;
        pairing::accept(stream, &store, invitation, &channel, &opened.boot_id).await
    }
    pub(crate) async fn sync_state(
        &self,
        boot: &str,
        registry: &crate::targets::Registry,
    ) -> Result<()> {
        let _sync = self.state_sync.lock().await;
        // Do not let a delayed update from an older control connection write
        // state into a newly established boot.
        if self.online_boot.borrow().as_deref() != Some(boot) {
            return Err(Error::new("HOST_OFFLINE", "publisher is reconnecting"));
        }
        let grants = Store::open(&self.profile, &self.origin)?
            .grants()?
            .allowed_here;
        let grants: Vec<_> = grants
            .into_iter()
            .map(|g| serde_json::json!({"device_id":g.device_id,"version":g.version}))
            .collect();
        let targets: Vec<_> = registry.snapshot().iter().map(|t| t.id.clone()).collect();
        let response: serde_json::Value = self
            .post(
                &format!("/v1/devices/{}/state", self.identity.id),
                &serde_json::json!({"boot_id":boot,"grants":grants,"targets":targets}),
            )
            .await?;
        if response["registered"] != true {
            return Err(protocol());
        }
        Ok(())
    }
    pub async fn publish(&self, boot: &str, registry: Arc<crate::targets::Registry>) -> Result<()> {
        self.state_ready
            .store(false, std::sync::atomic::Ordering::Release);
        let path = format!("/v1/devices/{}/control", self.identity.id);
        let mut socket = self.websocket(&path, "X-Relay-Boot", boot).await?;
        match event(&mut socket).await? {
            Event::Online { boot_id } if boot_id == boot => {}
            _ => return Err(protocol()),
        }
        self.online_boot.send_replace(Some(boot.to_owned()));
        self.sync_state(boot, &registry).await?;
        self.state_ready
            .store(true, std::sync::atomic::Ordering::Release);
        eprintln!("Relay online: {}", self.origin);
        let mut tasks = tokio::task::JoinSet::new();
        let mut ping = tokio::time::interval(Duration::from_secs(25));
        ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_seen = tokio::time::Instant::now();
        let result = async { loop {
            tokio::select! {
                _ = tokio::time::sleep_until(last_seen + Duration::from_secs(75)) => return Err(Error::new("NETWORK_LOST", "relay heartbeat timed out")),
                _ = ping.tick() => {
                    tokio::time::timeout(Duration::from_secs(10),socket.send(Message::Ping(Vec::new().into()))).await.map_err(|_| disconnected())?.map_err(socket_error)?;
                },
                _ = tasks.join_next(), if !tasks.is_empty() => {},
                message = socket.next() => {
                    match message.ok_or_else(disconnected)?.map_err(socket_error)? {
                        Message::Pong(_) => last_seen = tokio::time::Instant::now(),
                        Message::Ping(_) => tokio::time::timeout(Duration::from_secs(10),socket.flush()).await.map_err(|_| disconnected())?.map_err(socket_error)?,
                        Message::Text(text) => {
                            if text.len() > 8192 { return Err(protocol()); }
                            let event: Event = serde_json::from_str(&text).map_err(|_| protocol())?;
                            let (channel_id,connector,boot_id,ticket,purpose,target_id,grant_version,limits) = match event {
                                Event::Pair {channel_id,connector,boot_id,ticket} => (channel_id,connector,boot_id,ticket,Purpose::Pair,None,None,None),
                                Event::Access {channel_id,connector,boot_id,ticket,purpose,target_id,grant_version,limits} if purpose != Purpose::Pair => (channel_id,connector,boot_id,ticket,purpose,target_id,Some(grant_version),limits),
                                _ => return Err(protocol()),
                            };
                            if boot_id != boot || connector == self.identity.id { return Err(protocol()); }
                            while tasks.try_join_next().is_some() {}
                            if tasks.len() >= 12 { continue; }
                            let context = Context {version:1,origin:self.origin.clone(),publisher:self.identity.id.clone(),connector,purpose,grant_version,target:target_id,channel_id:channel_id.clone(),boot_id:boot_id.clone()};
                            context.validate()?;
                            let relay = self.clone();
                            let registry = registry.clone();
                            tasks.spawn(async move {
                                let result = async {
                                    let permit=relay.pending.clone().try_acquire_owned().map_err(|_|Error::new("QUOTA_EXCEEDED","four handshakes are already pending"))?;
                                    let stream = relay.join(&relay.identity.id,&channel_id,"publisher",&ticket,&boot_id,(context.purpose == Purpose::Attach).then_some(limits.unwrap_or_default())).await?;
                                    if context.purpose == Purpose::Pair {
                                        let store = Store::open(&relay.profile,&relay.origin)?;
                                        // Admit the committed grant before returning its receipt, so
                                        // the connector can immediately query shared targets.
                                        pairing::serve_after_commit(stream,&store,&channel_id,&boot_id,
                                            || relay.sync_state(&boot_id,&registry)).await
                                    } else {
                                        relay.serve_authorized(stream,&context,&registry,permit).await
                                    }
                                }.await;
                                if let Err(error) = result { eprintln!("Remote channel ended: {}",error.code); }
                            });
                        },
                        _ => return Err(disconnected()),
                    }
                }
            }
        }}.await;
        // Abort and await native/TLS operations before a new publisher boot.
        tasks.shutdown().await;
        result
    }
    async fn join(
        &self,
        publisher: &str,
        channel: &str,
        role: &str,
        ticket: &str,
        boot: &str,
        limits: Option<crate::stream::Limits>,
    ) -> Result<ChannelStream> {
        let mut socket = self
            .websocket(
                &format!("/v1/devices/{publisher}/channels/{channel}/{role}"),
                "X-Relay-Ticket",
                ticket,
            )
            .await?;
        match event(&mut socket).await? {
            Event::RouteReady {
                channel_id,
                boot_id,
            } if channel_id == channel && boot_id == boot => {}
            _ => return Err(protocol()),
        }
        if let Some(limits) = limits {
            let id: [u8; 16] = hex::decode(channel)
                .map_err(|_| protocol())?
                .try_into()
                .map_err(|_| protocol())?;
            Ok(Box::new(crate::stream::websocket_stream_with_limits(
                socket, &id, limits,
            )?))
        } else {
            Ok(Box::new(pair_stream(socket, channel)?))
        }
    }
    async fn access(
        &self,
        publisher: &str,
        target: Option<&str>,
        channel: &str,
    ) -> Result<tokio_rustls::client::TlsStream<ChannelStream>> {
        self.check_identity()?;
        let grant = host::grant(&self.profile, &self.origin, publisher, false)?;
        let purpose = if target.is_some() {
            Purpose::Attach
        } else {
            Purpose::Control
        };
        let opened: Opened = self
            .post(
                &format!("/v1/devices/{publisher}/access"),
                &serde_json::json!({"channel_id":channel,"purpose":purpose,"target_id":target}),
            )
            .await?;
        if opened.channel_id != channel {
            return Err(protocol());
        }
        if opened.grant_version != Some(grant.version) {
            return Err(Error::new(
                "NOT_AUTHORIZED",
                "host grant version changed; pair again if access was reissued",
            ));
        }
        let context = Context {
            version: 1,
            origin: self.origin.clone(),
            publisher: publisher.into(),
            connector: self.identity.id.clone(),
            purpose,
            grant_version: Some(grant.version),
            target: target.map(str::to_owned),
            channel_id: channel.to_owned(),
            boot_id: opened.boot_id.clone(),
        };
        context.validate()?;
        let stream = self
            .join(
                publisher,
                channel,
                "connector",
                &opened.ticket,
                &opened.boot_id,
                target.map(|_| opened.limits.unwrap_or_default()),
            )
            .await?;
        let certificate = hex::decode(&grant.certificate)
            .map_err(|_| protocol())?
            .into();
        crate::secure::connect(stream, &self.identity, &certificate, &context).await
    }
    pub(crate) fn authorization_changed(&self) {
        self.revocations
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    async fn serve_authorized(
        &self,
        stream: ChannelStream,
        context: &Context,
        registry: &crate::targets::Registry,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<()> {
        let mut changes = self.revocations.subscribe();
        let allowed = || {
            host::grant(&self.profile, &self.origin, &context.connector, true)
                .is_ok_and(|g| Some(g.version) == context.grant_version)
        };
        if !allowed() {
            return Err(Error::new(
                "NOT_AUTHORIZED",
                "device grant is no longer current",
            ));
        }
        let targets = registry.snapshot();
        let operation =
            host::serve_after_auth(stream, &self.profile, context, &targets, || async {
                let result: serde_json::Value = self
                    .post(
                        &format!(
                            "/v1/devices/{}/channels/{}/authenticated",
                            self.identity.id, context.channel_id
                        ),
                        &serde_json::json!({"boot_id":context.boot_id}),
                    )
                    .await?;
                if result["authenticated"] != true {
                    return Err(protocol());
                }
                drop(permit);
                Ok(())
            });
        tokio::pin!(operation);
        loop {
            tokio::select! {
                biased;
                _ = changes.changed() => {if !allowed() {return Err(Error::new("NOT_AUTHORIZED","device authorization ended"));}},
                result = &mut operation => return result,
            }
        }
    }
    pub async fn targets(&self, publisher: &str) -> Result<Vec<TargetStatus>> {
        let mut stream = self.access(publisher, None, &random_id()?).await?;
        let response: host::Response = control::receive(&mut stream).await?;
        let _ = control::send(&mut stream, &host::Ack {}).await;
        match response {
            host::Response::Targets { targets } => Ok(targets),
            host::Response::Rejected { code } => Err(remote_error(&code)),
            _ => Err(protocol()),
        }
    }
    pub async fn release_channel(&self, publisher: &str, channel: &str) -> Result<()> {
        let response: serde_json::Value = self
            .post(
                &format!("/v1/devices/{publisher}/channels/{channel}/release"),
                &serde_json::json!({}),
            )
            .await?;
        if response["released"] != true {
            return Err(protocol());
        }
        Ok(())
    }
    pub async fn prepare_attach(
        &self,
        publisher: &str,
        target: &str,
        channel: &str,
    ) -> Result<PreparedAttach> {
        let mut stream = self.access(publisher, Some(target), channel).await?;
        match control::receive(&mut stream).await? {
            host::Response::Ready { runtime_id } => Ok(PreparedAttach { stream, runtime_id }),
            host::Response::Rejected { code } => {
                let _ = control::send(&mut stream, &host::Ack {}).await;
                Err(remote_error(&code))
            }
            _ => Err(protocol()),
        }
    }
}

pub struct PreparedAttach {
    stream: tokio_rustls::client::TlsStream<ChannelStream>,
    runtime_id: String,
}
impl PreparedAttach {
    pub fn runtime_id(&self) -> &str {
        &self.runtime_id
    }
    pub async fn start(mut self) -> Result<tokio_rustls::client::TlsStream<ChannelStream>> {
        control::send(
            &mut self.stream,
            &host::Start {
                runtime_id: self.runtime_id,
            },
        )
        .await?;
        match control::receive(&mut self.stream).await? {
            host::Response::Started => Ok(self.stream),
            host::Response::Rejected { code } => {
                let _ = control::send(&mut self.stream, &host::Ack {}).await;
                Err(remote_error(&code))
            }
            _ => Err(protocol()),
        }
    }
}

async fn response_json<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T> {
    let success = response.status().is_success();
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get("Retry-After")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| crate::retry::retry_after(v, std::time::SystemTime::now()));
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network)? {
        if body.len() + chunk.len() > 8192 {
            return Err(protocol());
        }
        body.extend_from_slice(&chunk);
    }
    if !success {
        #[derive(Deserialize)]
        struct Failure {
            code: String,
        }
        let code = serde_json::from_slice::<Failure>(&body)
            .map(|v| v.code)
            .unwrap_or_default();
        let mut error = remote_error(&code);
        if code.is_empty() {
            error = http_error(status);
        }
        error.retry_after = retry_after;
        return Err(error);
    }
    serde_json::from_slice(&body).map_err(|_| protocol())
}
async fn event(socket: &mut Socket) -> Result<Event> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match socket
                .next()
                .await
                .ok_or_else(disconnected)?
                .map_err(socket_error)?
            {
                Message::Text(text) if text.len() <= 8192 => {
                    return serde_json::from_str(&text).map_err(|_| protocol());
                }
                Message::Ping(_) => socket.flush().await.map_err(socket_error)?,
                Message::Pong(_) => {}
                _ => return Err(protocol()),
            }
        }
    })
    .await
    .map_err(|_| Error::new("NETWORK_LOST", "relay route setup timed out"))?
}
fn pair_stream(mut socket: Socket, channel: &str) -> Result<tokio::io::DuplexStream> {
    let channel: [u8; 16] = hex::decode(channel)
        .map_err(|_| protocol())?
        .try_into()
        .map_err(|_| protocol())?;
    let (client, bridge) = tokio::io::duplex(16 * 1024);
    tokio::spawn(async move {
        let pump = async {
            let (mut reader, mut writer) = tokio::io::split(bridge);
            let mut bytes = [0; 16384];
            let (mut sent, mut received, mut ack) = (0u64, 0u64, 0u64);
            let mut messages = 0;
            loop {
                tokio::select! {
                    read = reader.read(&mut bytes) => {
                        let size = read.map_err(local_io)?;
                        if size == 0 { let _ = socket.close(None).await; return Ok(()); }
                        if sent+size as u64 > 65536 { return Err(protocol()); }
                        let frame = Frame {kind:Kind::Data,channel,offset:sent,payload:&bytes[..size]}.encode()?;
                        socket.send(Message::Binary(frame.into())).await.map_err(socket_error)?;
                        sent += size as u64;
                    },
                    message = socket.next() => match message.ok_or_else(disconnected)?.map_err(socket_error)? {
                        Message::Binary(bytes) => {
                            messages += 1;
                            let frame = Frame::decode(&bytes)?;
                            if frame.channel != channel || messages > 256 { return Err(protocol()); }
                            match frame.kind {
                                Kind::Data => {
                                    if frame.offset != received || received+frame.payload.len() as u64 > 65536 { return Err(protocol()); }
                                    writer.write_all(frame.payload).await.map_err(local_io)?;
                                    received += frame.payload.len() as u64;
                                    let credit = Frame {kind:Kind::Credit,channel,offset:received,payload:&[]}.encode()?;
                                    socket.send(Message::Binary(credit.into())).await.map_err(socket_error)?;
                                },
                                Kind::Credit => { if frame.offset < ack || frame.offset > sent { return Err(protocol()); } ack = frame.offset; }
                            }
                        },
                        Message::Ping(_) => socket.flush().await.map_err(socket_error)?,
                        Message::Pong(_) => {},
                        _ => return Err(disconnected()),
                    }
                }
            }
        };
        // Pairing has a finite transcript; this adapter is not a native stream.
        let _ = tokio::time::timeout(Duration::from_secs(30), pump).await;
    });
    Ok(client)
}
fn protocol() -> Error {
    Error::new("PROTOCOL_ERROR", "invalid relay control message or frame")
}
fn disconnected() -> Error {
    Error::new("NETWORK_LOST", "relay connection closed")
}
fn network(error: reqwest::Error) -> Error {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = cause {
        if current.is::<rustls::Error>() {
            return Error::new(
                "TLS_AUTH_FAILED",
                "relay HTTPS certificate or TLS authentication failed",
            );
        }
        cause = current.source();
    }
    Error::new(
        "NETWORK_LOST",
        "relay request failed; check connectivity and relay TLS certificate",
    )
}
fn local_io(error: std::io::Error) -> Error {
    Error::new("CONNECTION_LOST", error.to_string())
}
fn remote_error(code: &str) -> Error {
    let code = match code {
        "HOST_OFFLINE" => "HOST_OFFLINE",
        "TARGET_BUSY" => "TARGET_BUSY",
        "PROTOCOL_ERROR" => "PROTOCOL_ERROR",
        "INTERNAL_ERROR" => "INTERNAL_ERROR",
        "DEVELOPMENT_ONLY" => "DEVELOPMENT_ONLY",
        "TARGET_OFFLINE" => "TARGET_OFFLINE",
        "TARGET_NOT_SHARED" => "TARGET_NOT_SHARED",
        "CHANNEL_EXPIRED" => "CHANNEL_EXPIRED",
        "TARGET_CHANGED" => "TARGET_CHANGED",
        "PAIR_INVALID" => "PAIR_INVALID",
        "ACCESS_INVALID" => "ACCESS_INVALID",
        "RATE_LIMITED" => "RATE_LIMITED",
        "QUOTA_EXCEEDED" => "QUOTA_EXCEEDED",
        "GLOBAL_QUOTA_EXCEEDED" => "GLOBAL_QUOTA_EXCEEDED",
        "CHANNEL_CONFLICT" => "CHANNEL_CONFLICT",
        "TICKET_INVALID" => "TICKET_INVALID",
        "RELAY_BUSY" => "RELAY_BUSY",
        "NOT_AUTHORIZED" => "NOT_AUTHORIZED",
        _ => "RELAY_ERROR",
    };
    Error::new(
        code,
        if code == "GLOBAL_QUOTA_EXCEEDED" {
            "the relay's global capacity or daily service quota is exhausted; retry later or choose a self-hosted relay"
        } else {
            "relay rejected the request"
        },
    )
}
fn http_error(status: u16) -> Error {
    match status {
        429 => remote_error("RATE_LIMITED"),
        500..=599 => remote_error("RELAY_BUSY"),
        _ => remote_error("RELAY_ERROR"),
    }
}
fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> Error {
    if matches!(error, tokio_tungstenite::tungstenite::Error::Tls(_)) {
        return Error::new(
            "TLS_AUTH_FAILED",
            "relay WebSocket TLS authentication failed",
        );
    }
    if let tokio_tungstenite::tungstenite::Error::Http(response) = error {
        let mut error = response
            .body()
            .as_ref()
            .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
            .and_then(|value| value["code"].as_str().map(remote_error))
            .unwrap_or_else(|| http_error(response.status().as_u16()));
        error.retry_after = response
            .headers()
            .get("Retry-After")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| crate::retry::retry_after(v, std::time::SystemTime::now()));
        return error;
    }
    disconnected()
}
