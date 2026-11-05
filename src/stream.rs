//! Bounded ciphertext transport. Credit is returned only after the endpoint
//! consumes bytes; closing either endpoint discards all pending bytes.
use crate::{
    Error, Result,
    frame::{Frame, Kind},
};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

/// Legacy window, used unless the relay negotiated stream v2 for both ends.
pub const WINDOW: u64 = 65536;
pub const MAX_WINDOW: u32 = 1048576;
pub const MAX_BYTES_PER_SECOND: u32 = 4194304;
/// Advertised to the relay; it enables stream v2 only when both ends do.
pub const FEATURES: &str = "stream-v2,prepared-join,control-authenticated";

/// A present `window` means stream v2: that window, a rate above the legacy
/// 256 KiB/s cap, and a separate credit message budget.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub bytes_per_second: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<u32>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes_per_second: 262144,
            window: None,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        let rate = match self.window {
            None => 16384..=262144,
            Some(window) if (WINDOW as u32..=MAX_WINDOW).contains(&window) => {
                16384..=MAX_BYTES_PER_SECOND
            }
            Some(_) => return Err(Error::new("PROTOCOL_ERROR", "unsupported stream window")),
        };
        if !rate.contains(&self.bytes_per_second) {
            return Err(Error::new(
                "PROTOCOL_ERROR",
                "unsupported stream rate limit",
            ));
        }
        Ok(())
    }
    fn window(&self) -> u64 {
        self.window.map_or(WINDOW, u64::from)
    }
}

struct Budget {
    bytes: f64,
    messages: f64,
    /// Separate credit messages for stream v2; `None` shares `messages`.
    credits: Option<f64>,
    byte_rate: f64,
    message_rate: f64,
    message_burst: f64,
    burst: f64,
    updated: tokio::time::Instant,
}
impl Budget {
    fn new(limits: Limits, pace: f64, message_burst: f64) -> Self {
        let burst = (limits.bytes_per_second as f64 * 2.0).max(limits.window() as f64);
        Self {
            bytes: burst,
            messages: message_burst,
            credits: limits.window.map(|_| message_burst),
            byte_rate: limits.bytes_per_second as f64 * pace,
            message_rate: 150.0 * pace,
            message_burst,
            burst,
            updated: tokio::time::Instant::now(),
        }
    }
    fn refill(&mut self) {
        let now = tokio::time::Instant::now();
        let elapsed = (now - self.updated).as_secs_f64();
        self.updated = now;
        self.bytes = (self.bytes + elapsed * self.byte_rate).min(self.burst);
        self.messages = (self.messages + elapsed * self.message_rate).min(self.message_burst);
        if let Some(credits) = &mut self.credits {
            *credits = (*credits + elapsed * self.message_rate).min(self.message_burst);
        }
    }
    fn credit_messages(&self) -> f64 {
        self.credits.unwrap_or(self.messages)
    }
    fn credit_ready(&self) -> bool {
        self.credit_messages() >= 1.0
    }
    fn take_credit(&mut self) -> Result<()> {
        self.refill();
        if !self.credit_ready() {
            return Err(protocol());
        }
        match &mut self.credits {
            Some(credits) => *credits -= 1.0,
            None => self.messages -= 1.0,
        }
        Ok(())
    }
    fn credit_deadline(&self) -> tokio::time::Instant {
        self.updated
            + Duration::from_secs_f64(((1.0 - self.credit_messages()) / self.message_rate).max(0.0))
                .max(Duration::from_micros(1))
    }
    /// Validate a received frame against the peer's pace.
    fn take_frame(&mut self, kind: Kind, bytes: usize) -> Result<()> {
        if kind == Kind::Credit && self.credits.is_some() {
            return self.take_credit();
        }
        self.take(bytes)
    }
    fn ready(&self, bytes: usize) -> bool {
        self.messages >= 1.0 && self.bytes >= bytes as f64
    }
    fn take(&mut self, bytes: usize) -> Result<()> {
        self.refill();
        if !self.ready(bytes) {
            return Err(protocol());
        }
        self.messages -= 1.0;
        self.bytes -= bytes as f64;
        Ok(())
    }
    fn deadline(&self, bytes: usize) -> tokio::time::Instant {
        self.updated
            + Duration::from_secs_f64(
                ((bytes as f64 - self.bytes) / self.byte_rate)
                    .max((1.0 - self.messages) / self.message_rate)
                    .max(0.0),
            )
            .max(Duration::from_micros(1))
    }
}

pub fn websocket_stream<S>(socket: WebSocketStream<S>, channel: &[u8; 16]) -> Stream
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    websocket_stream_with_limits(socket, channel, Limits::default())
        .expect("default stream limits are valid")
}

pub fn websocket_stream_with_limits<S>(
    mut socket: WebSocketStream<S>,
    channel: &[u8; 16],
    limits: Limits,
) -> Result<Stream>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    limits.validate()?;
    let channel = *channel;
    let (client, bridge) = tokio::io::duplex(16384);
    let (cancel, mut cancelled) = tokio::sync::oneshot::channel();
    let (failure_tx, failure) = tokio::sync::watch::channel(None);
    tokio::spawn(async move {
        let (mut reader, mut writer) = tokio::io::split(bridge);
        let (incoming, mut queued) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        let (consumed_tx, mut consumed_rx) = tokio::sync::watch::channel(0u64);
        let write_task = tokio::spawn(async move {
            let mut consumed = 0;
            while let Some(bytes) = queued.recv().await {
                if writer.write_all(&bytes).await.is_err() {
                    break;
                }
                consumed += bytes.len() as u64;
                if consumed_tx.send(consumed).is_err() {
                    break;
                }
            }
        });
        let pump = async {
            let (mut sent, mut ack, mut received, mut credited, mut consumed) =
                (0u64, 0u64, 0u64, 0u64, 0u64);
            // Leave one second of message headroom for network batching. A
            // slower refill alone cannot help when both ends start with the
            // same full burst: 300 messages plus the first refilled token can
            // arrive together and exceed the relay's 300-message allowance.
            // Keep the byte window and sustained rate unchanged.
            let window = limits.window();
            let mut outgoing = Budget::new(limits, 0.95, 150.0);
            let mut incoming_budget = Budget::new(limits, 1.0, 300.0);
            let mut bytes = [0; 16384];
            let mut ping = tokio::time::interval(Duration::from_secs(25));
            let mut last_seen = tokio::time::Instant::now();
            let mut send_progress = last_seen;
            let mut receive_progress = last_seen;
            // Bytes already read from the endpoint, sent once the budget covers
            // their actual size: a keystroke never waits for a full chunk.
            let mut pending = 0usize;
            loop {
                outgoing.refill();
                let allowance = (window - (sent - ack)).min(16384) as usize;
                let can_data = pending > 0 && outgoing.ready(pending);
                let can_credit = outgoing.credit_ready();
                let budget_deadline = if consumed > credited && !can_credit {
                    outgoing.credit_deadline()
                } else {
                    outgoing.deadline(pending)
                };
                tokio::select! {
                    biased;
                    _ = &mut cancelled => return Ok(()),
                    _ = tokio::time::sleep_until(last_seen + Duration::from_secs(75)) => return Err(lost("relay heartbeat timed out after 75 seconds")),
                    _ = tokio::time::sleep_until(send_progress + Duration::from_secs(30)), if sent > ack => return Err(lost("relay peer did not acknowledge data for 30 seconds")),
                    _ = tokio::time::sleep_until(receive_progress + Duration::from_secs(30)), if received > consumed => return Err(lost("local consumer stalled for 30 seconds")),
                    _ = tokio::time::sleep_until(budget_deadline), if (pending > 0 && !can_data) || (consumed>credited && !can_credit) => {},
                    _ = async {}, if consumed>credited && can_credit => {
                        outgoing.take_credit()?;
                        let frame = Frame {kind:Kind::Credit,channel,offset:consumed,payload:&[]}.encode()?;
                        send(&mut socket,Message::Binary(frame.into())).await?;
                        credited=consumed;
                    },
                    _ = async {}, if can_data => {
                        if sent == ack { send_progress = tokio::time::Instant::now(); }
                        outgoing.take(pending)?;
                        let frame = Frame {kind:Kind::Data,channel,offset:sent,payload:&bytes[..pending]}.encode()?;
                        send(&mut socket,Message::Binary(frame.into())).await?;
                        sent = sent.checked_add(pending as u64).ok_or_else(protocol)?;
                        pending = 0;
                    },
                    result = reader.read(&mut bytes[..allowance]), if allowance > 0 && pending == 0 => {
                        let n = result.map_err(io)?;
                        if n == 0 { return Ok(()); }
                        pending = n;
                    },
                    result = consumed_rx.changed() => {
                        result.map_err(|_| protocol())?;
                        consumed = *consumed_rx.borrow_and_update();
                        receive_progress = tokio::time::Instant::now();
                    },
                    // This pump is the queue's only sender. Wait for the writer's
                    // consumption notification before reading more frames when
                    // it is full; a valid burst is backpressure, not a protocol
                    // violation. Cancellation and progress deadlines stay live.
                    message = socket.next(), if incoming.capacity() > 0 => {
                        last_seen = tokio::time::Instant::now();
                        match message.ok_or_else(|| lost("relay WebSocket ended without a close frame"))?.map_err(socket_error)? {
                            Message::Binary(bytes) => {
                                let frame = Frame::decode(&bytes)?;
                                incoming_budget.take_frame(frame.kind, frame.payload.len())
                                    .map_err(|_| lost("received frames exceeded the relay stream rate limit"))?;
                                if frame.channel != channel { return Err(protocol()); }
                                match frame.kind {
                                    Kind::Data => {
                                        let end = received.checked_add(frame.payload.len() as u64).ok_or_else(protocol)?;
                                        if frame.payload.is_empty() || frame.offset != received || end-credited > window { return Err(protocol()); }
                                        if received == consumed { receive_progress = tokio::time::Instant::now(); }
                                        incoming.try_send(frame.payload.to_vec()).map_err(|_| protocol())?;
                                        received = end;
                                    },
                                    Kind::Credit => {
                                        if frame.offset < ack || frame.offset > sent { return Err(protocol()); }
                                        if frame.offset > ack { send_progress = tokio::time::Instant::now(); }
                                        ack = frame.offset;
                                    }
                                }
                            },
                            Message::Ping(_) => { tokio::time::timeout(Duration::from_secs(10),socket.flush()).await.map_err(|_| lost("relay WebSocket flush timed out after 10 seconds"))?.map_err(socket_error)?; },
                            Message::Pong(_) => {},
                            Message::Close(Some(frame)) if u16::from(frame.code) != 1000 => return Err(close_error(u16::from(frame.code), frame.reason.as_str())),
                            _ => return Ok(()),
                        }
                    },
                    _ = ping.tick() => {
                        send(&mut socket,Message::Ping(Vec::new().into())).await?;
                    }
                }
            }
        };
        let result: Result<()> = pump.await;
        if let Err(error) = result {
            // Publish the cause before dropping the bridge. The native/TLS
            // reader must see the transport failure instead of an anonymous EOF.
            failure_tx.send_replace(Some(error.message));
        }
        write_task.abort();
        let _ = write_task.await;
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
    });
    Ok(Stream {
        inner: client,
        cancel: Some(cancel),
        failure,
    })
}
async fn send<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: Message,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), socket.send(message))
        .await
        .map_err(|_| lost("relay WebSocket send timed out after 10 seconds"))?
        .map_err(socket_error)
}
fn lost(message: impl Into<String>) -> Error {
    Error::new("CONNECTION_LOST", message)
}
fn socket_error(error: tokio_tungstenite::tungstenite::Error) -> Error {
    use tokio_tungstenite::tungstenite::Error as SocketError;
    match error {
        SocketError::Io(error) => lost(format!("relay socket I/O failed: {error}")),
        SocketError::Tls(_) => lost("relay socket TLS failed"),
        SocketError::Protocol(_) => lost("relay WebSocket protocol error or unclean close"),
        _ => lost("relay WebSocket failed"),
    }
}
fn close_error(code: u16, reason: &str) -> Error {
    // Relay reasons are untrusted text. Keep only known protocol codes; never
    // echo arbitrary close text (which could contain secrets or terminal escapes).
    if code == 4000
        && matches!(
            reason,
            "RATE_LIMITED"
                | "SLOW_CONSUMER"
                | "HOST_OFFLINE"
                | "CONNECTION_LOST"
                | "CHANNEL_EXPIRED"
                | "GLOBAL_QUOTA_EXCEEDED"
                | "RELAY_BUSY"
                | "PROTOCOL_ERROR"
                | "STATE_CONFLICT"
                | "PROTOCOL_CHANGED"
                | "NOT_AUTHORIZED"
                | "TARGET_NOT_SHARED"
                | "TARGET_OFFLINE"
        )
    {
        lost(format!("relay closed stream ({reason})"))
    } else {
        lost(format!("relay closed stream (WebSocket code {code})"))
    }
}
fn protocol() -> Error {
    Error::new(
        "CONNECTION_LOST",
        "ciphertext stream closed or violated its credit window",
    )
}
fn io(error: std::io::Error) -> Error {
    Error::new("CONNECTION_LOST", error.to_string())
}

/// Dropping a stream cancels its pump even when all sending credit is spent.
pub struct Stream {
    inner: tokio::io::DuplexStream,
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
    failure: tokio::sync::watch::Receiver<Option<String>>,
}
impl Stream {
    fn failure(&self) -> Option<std::io::Error> {
        self.failure.borrow().as_ref().map(|message| {
            std::io::Error::new(std::io::ErrorKind::ConnectionAborted, message.clone())
        })
    }
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let has_capacity = buf.remaining() > 0;
        match std::pin::Pin::new(&mut self.inner).poll_read(cx, buf) {
            std::task::Poll::Ready(Ok(())) if has_capacity && buf.filled().len() == before => {
                std::task::Poll::Ready(self.failure().map_or(Ok(()), Err))
            }
            std::task::Poll::Ready(Err(error)) => {
                std::task::Poll::Ready(Err(self.failure().unwrap_or(error)))
            }
            result => result,
        }
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner)
            .poll_write(cx, bytes)
            .map_err(|error| self.failure().unwrap_or(error))
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner)
            .poll_flush(cx)
            .map_err(|error| self.failure().unwrap_or(error))
    }
    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if let Some(cancel) = self.cancel.take() {
            let _ = cancel.send(());
        }
        std::pin::Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
