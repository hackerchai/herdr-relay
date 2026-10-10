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

pub const WINDOW: u64 = 65536;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub bytes_per_second: u32,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            bytes_per_second: 262144,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<()> {
        if !(16384..=262144).contains(&self.bytes_per_second) {
            return Err(Error::new(
                "PROTOCOL_ERROR",
                "unsupported stream rate limit",
            ));
        }
        Ok(())
    }
}

struct Budget {
    bytes: f64,
    messages: f64,
    byte_rate: f64,
    message_rate: f64,
    burst: f64,
    updated: tokio::time::Instant,
}
impl Budget {
    fn new(limits: Limits, pace: f64) -> Self {
        let burst = (limits.bytes_per_second as f64 * 2.0).max(WINDOW as f64);
        Self {
            bytes: burst,
            messages: 300.0,
            byte_rate: limits.bytes_per_second as f64 * pace,
            message_rate: 150.0 * pace,
            burst,
            updated: tokio::time::Instant::now(),
        }
    }
    fn refill(&mut self) {
        let now = tokio::time::Instant::now();
        let elapsed = (now - self.updated).as_secs_f64();
        self.updated = now;
        self.bytes = (self.bytes + elapsed * self.byte_rate).min(self.burst);
        self.messages = (self.messages + elapsed * self.message_rate).min(300.0);
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
            // A small pacing margin tolerates network batching and clock
            // differences while the cloud still enforces the advertised cap.
            let mut outgoing = Budget::new(limits, 0.95);
            let mut incoming_budget = Budget::new(limits, 1.0);
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
                let allowance = (WINDOW - (sent - ack)).min(16384) as usize;
                let can_data = pending > 0 && outgoing.ready(pending);
                let can_credit = outgoing.ready(0);
                let budget_deadline = outgoing.deadline(if consumed > credited && !can_credit {
                    0
                } else {
                    pending
                });
                tokio::select! {
                    biased;
                    _ = &mut cancelled => return Ok(()),
                    _ = tokio::time::sleep_until(last_seen + Duration::from_secs(75)) => return Err(protocol()),
                    _ = tokio::time::sleep_until(send_progress + Duration::from_secs(30)), if sent > ack => return Err(protocol()),
                    _ = tokio::time::sleep_until(receive_progress + Duration::from_secs(30)), if received > consumed => return Err(protocol()),
                    _ = tokio::time::sleep_until(budget_deadline), if (pending > 0 && !can_data) || (consumed>credited && !can_credit) => {},
                    _ = async {}, if consumed>credited && can_credit => {
                        outgoing.take(0)?;
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
                    message = socket.next() => {
                        last_seen = tokio::time::Instant::now();
                        match message.ok_or_else(protocol)?.map_err(|_| protocol())? {
                            Message::Binary(bytes) => {
                                let frame = Frame::decode(&bytes)?;
                                incoming_budget.take(frame.payload.len())?;
                                if frame.channel != channel { return Err(protocol()); }
                                match frame.kind {
                                    Kind::Data => {
                                        let end = received.checked_add(frame.payload.len() as u64).ok_or_else(protocol)?;
                                        if frame.payload.is_empty() || frame.offset != received || end-credited > WINDOW { return Err(protocol()); }
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
                            Message::Ping(_) => { tokio::time::timeout(Duration::from_secs(10),socket.flush()).await.map_err(|_| protocol())?.map_err(|_| protocol())?; },
                            Message::Pong(_) => {},
                            _ => return Ok(()),
                        }
                    },
                    _ = ping.tick() => {
                        send(&mut socket,Message::Ping(Vec::new().into())).await?;
                    }
                }
            }
        };
        let _: Result<()> = pump.await;
        write_task.abort();
        let _ = write_task.await;
        let _ = tokio::time::timeout(Duration::from_secs(1), socket.close(None)).await;
    });
    Ok(Stream {
        inner: client,
        cancel: Some(cancel),
    })
}
async fn send<S: AsyncRead + AsyncWrite + Unpin>(
    socket: &mut WebSocketStream<S>,
    message: Message,
) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(10), socket.send(message))
        .await
        .map_err(|_| protocol())?
        .map_err(|_| protocol())
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
}
impl AsyncRead for Stream {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}
impl AsyncWrite for Stream {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.inner).poll_write(cx, bytes)
    }
    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.inner).poll_flush(cx)
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
