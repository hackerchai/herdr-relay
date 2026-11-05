use herdr_relay::stream::{Limits, Stream, websocket_stream, websocket_stream_with_limits};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

#[tokio::test]
async fn a_batched_burst_of_small_frames_does_not_disconnect_a_healthy_reader() {
    for limits in [Limits::default(), v2()] {
        for (count, size) in [(64, 1), (65, 1), (128, 64), (300, 64), (128, 512)] {
            let (mut stream, _peer) = buffered_burst(count, size, limits).await;
            let mut received = vec![0; count * size];
            tokio::time::timeout(Duration::from_secs(1), stream.read_exact(&mut received))
                .await
                .expect("a healthy reader must keep making progress")
                .unwrap_or_else(|error| {
                    panic!("{count} valid batched frames disconnected the stream: {error}")
                });
            assert!(received.iter().all(|&byte| byte == 0x42));
        }
    }
}

async fn buffered_burst(
    count: usize,
    size: usize,
    limits: Limits,
) -> (Stream, WebSocketStream<tokio::io::DuplexStream>) {
    use futures_util::SinkExt;
    use herdr_relay::frame::{Frame, Kind};
    use tokio_tungstenite::tungstenite::Message;

    // A network read may deliver many small screen updates together. These
    // frames fit both the advertised byte window and the 300-message burst.
    let (a, b) = tokio::io::duplex(1024 * 1024);
    let a = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
    let mut peer = WebSocketStream::from_raw_socket(b, Role::Server, None).await;
    let channel = [8; 16];
    let payload = vec![0x42; size];
    for offset in (0..count * payload.len()).step_by(payload.len()) {
        let bytes = Frame {
            kind: Kind::Data,
            channel,
            offset: offset as u64,
            payload: &payload,
        }
        .encode()
        .unwrap();
        peer.feed(Message::Binary(bytes.into())).await.unwrap();
    }
    peer.flush().await.unwrap();
    (
        websocket_stream_with_limits(a, &channel, limits).unwrap(),
        peer,
    )
}

// Read until close, answering heartbeats while the native consumer is paused.
async fn peer_closed(peer: &mut WebSocketStream<tokio::io::DuplexStream>) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    loop {
        match peer.next().await {
            Some(Ok(Message::Ping(_))) => peer.flush().await.unwrap(),
            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
            _ => {}
        }
    }
}

#[tokio::test(start_paused = true)]
async fn a_full_receive_queue_recovers_after_a_short_consumer_pause() {
    let (mut stream, mut peer) = buffered_burst(128, 512, Limits::default()).await;
    // 64 KiB fills the 16 KiB endpoint buffer and the 64-frame receive queue.
    assert!(
        tokio::time::timeout(Duration::from_secs(1), peer_closed(&mut peer))
            .await
            .is_err()
    );
    let mut received = vec![0; 65536];
    tokio::time::timeout(Duration::from_secs(1), stream.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert!(received.iter().all(|&byte| byte == 0x42));
}

#[tokio::test(start_paused = true)]
async fn a_full_receive_queue_still_expires_when_consumption_stalls() {
    let (_stream, mut peer) = buffered_burst(128, 512, Limits::default()).await;
    let began = tokio::time::Instant::now();
    assert!(
        tokio::time::timeout(Duration::from_secs(1), peer_closed(&mut peer))
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(30), peer_closed(&mut peer))
        .await
        .expect("a full queue must not bypass the consumption deadline");
    assert!(began.elapsed() >= Duration::from_secs(30));
}

#[tokio::test(start_paused = true)]
async fn dropping_a_full_receive_queue_closes_without_waiting_for_consumption() {
    let (stream, mut peer) = buffered_burst(128, 512, Limits::default()).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(1), peer_closed(&mut peer))
            .await
            .is_err()
    );
    drop(stream);
    tokio::time::timeout(Duration::from_secs(1), peer_closed(&mut peer))
        .await
        .expect("a full receive queue must remain cancellable");
}

#[tokio::test]
async fn credit_window_carries_simultaneous_large_streams_through_slow_readers() {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let (a, b) = tokio::io::duplex(32768);
        let (a, b) = tokio::join!(
            WebSocketStream::from_raw_socket(a, Role::Client, None),
            WebSocketStream::from_raw_socket(b, Role::Server, None)
        );
        let a = websocket_stream(a, &[1; 16]);
        let b = websocket_stream(b, &[1; 16]);
        let (mut ar, mut aw) = tokio::io::split(a);
        let (mut br, mut bw) = tokio::io::split(b);
        let send_a = async { aw.write_all(&vec![0x42; 1024 * 1024]).await.unwrap() };
        let send_b = async { bw.write_all(&vec![0xa7; 1024 * 1024]).await.unwrap() };
        let read_a = async {
            for _ in 0..1024 {
                let mut data = [0; 1024];
                ar.read_exact(&mut data).await.unwrap();
                assert!(data.iter().all(|&b| b == 0xa7));
                tokio::task::yield_now().await;
            }
        };
        let read_b = async {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            for _ in 0..1024 {
                let mut data = [0; 1024];
                br.read_exact(&mut data).await.unwrap();
                assert!(data.iter().all(|&b| b == 0x42));
                tokio::task::yield_now().await;
            }
        };
        tokio::join!(send_a, send_b, read_a, read_b);
        drop(aw);
        drop(ar);
        assert_eq!(br.read(&mut [0]).await.unwrap(), 0);
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dropping_a_full_credit_window_closes_without_waiting_for_peer_credit() {
    use futures_util::StreamExt;
    let (a, b) = tokio::io::duplex(1024 * 1024);
    let a = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
    let mut b = WebSocketStream::from_raw_socket(b, Role::Server, None).await;
    let mut stream = websocket_stream(a, &[2; 16]);
    stream.write_all(&vec![1; 65536 + 16384]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while let Some(Ok(message)) = b.next().await {
            if matches!(message, tokio_tungstenite::tungstenite::Message::Close(_)) {
                return;
            }
        }
    })
    .await
    .expect("closing a full stream waited for unavailable credit");
}

#[tokio::test(start_paused = true)]
async fn a_peer_that_answers_pings_but_never_consumes_data_cannot_hold_a_channel_forever() {
    use futures_util::{SinkExt, StreamExt};
    use herdr_relay::frame::{Frame, Kind};
    use tokio_tungstenite::tungstenite::Message;
    let (a, b) = tokio::io::duplex(1024 * 1024);
    let a = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
    let mut peer = WebSocketStream::from_raw_socket(b, Role::Server, None).await;
    let mut stream = websocket_stream(a, &[3; 16]);
    stream.write_all(&vec![1; 65536]).await.unwrap();
    let mut received = 0;
    while received < 65536 {
        match peer.next().await.unwrap().unwrap() {
            Message::Binary(bytes) => {
                let frame = Frame::decode(&bytes).unwrap();
                if frame.kind == Kind::Data {
                    received += frame.payload.len();
                }
            }
            Message::Ping(_) => peer.flush().await.unwrap(),
            _ => {}
        }
    }
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match peer.next().await {
                Some(Ok(Message::Ping(_))) => {
                    if peer.flush().await.is_err() {
                        break;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Err(_)) => break,
                _ => {}
            }
        }
    })
    .await
    .expect("a stalled credit window survived its 30-second progress deadline");
}

#[tokio::test(start_paused = true)]
async fn a_keystroke_after_a_burst_does_not_wait_for_a_full_chunk_of_budget() {
    let (a, b) = tokio::io::duplex(1024 * 1024);
    let (a, b) = tokio::join!(
        WebSocketStream::from_raw_socket(a, Role::Client, None),
        WebSocketStream::from_raw_socket(b, Role::Server, None)
    );
    let (_ar, mut aw) = tokio::io::split(websocket_stream(a, &[4; 16]));
    let (mut br, _bw) = tokio::io::split(websocket_stream(b, &[4; 16]));
    // The default 256 KiB/s limit allows a 512 KiB burst; spend all of it.
    let burst = 512 * 1024;
    let send = async { aw.write_all(&vec![1; burst]).await.unwrap() };
    let receive = async {
        let mut data = vec![0; burst];
        br.read_exact(&mut data).await.unwrap();
    };
    tokio::join!(send, receive);
    let began = tokio::time::Instant::now();
    aw.write_all(b"k").await.unwrap();
    let mut key = [0; 1];
    br.read_exact(&mut key).await.unwrap();
    assert_eq!(&key, b"k");
    let waited = began.elapsed();
    // Refilling one byte takes microseconds; a full 16 KiB chunk takes ~66 ms.
    assert!(
        waited < std::time::Duration::from_millis(5),
        "a one-byte message waited {waited:?} for unrelated budget"
    );
}

fn v2() -> herdr_relay::stream::Limits {
    herdr_relay::stream::Limits {
        bytes_per_second: 1048576,
        window: Some(262144),
    }
}

/// Collect Data payload bytes a raw peer receives until nothing arrives for 50 ms.
async fn drain_data<S>(peer: &mut WebSocketStream<S>) -> (usize, Vec<u64>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use futures_util::StreamExt;
    use herdr_relay::frame::{Frame, Kind};
    use tokio_tungstenite::tungstenite::Message;
    let (mut data, mut credits) = (0, vec![]);
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(std::time::Duration::from_millis(50), peer.next()).await
    {
        if let Message::Binary(bytes) = message {
            let frame = Frame::decode(&bytes).unwrap();
            match frame.kind {
                Kind::Data => data += frame.payload.len(),
                Kind::Credit => credits.push(frame.offset),
            }
        }
    }
    (data, credits)
}

#[tokio::test(start_paused = true)]
async fn a_negotiated_window_keeps_more_data_in_flight_than_the_legacy_window() {
    for (limits, expected) in [
        (herdr_relay::stream::Limits::default(), 65536),
        (v2(), 262144),
    ] {
        let (a, b) = tokio::io::duplex(4 * 1024 * 1024);
        let a = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
        let mut peer = WebSocketStream::from_raw_socket(b, Role::Server, None).await;
        let mut stream =
            herdr_relay::stream::websocket_stream_with_limits(a, &[5; 16], limits).unwrap();
        let writer = tokio::spawn(async move {
            let _ = stream.write_all(&vec![1; 512 * 1024]).await;
            stream
        });
        let (data, _) = drain_data(&mut peer).await;
        assert_eq!(data, expected);
        writer.abort();
    }
}

#[tokio::test(start_paused = true)]
async fn credits_are_not_delayed_by_spent_data_messages_once_v2_is_negotiated() {
    use futures_util::{SinkExt, StreamExt};
    use herdr_relay::frame::{Frame, Kind};
    use tokio_tungstenite::tungstenite::Message;
    for (limits, delayed) in [
        (herdr_relay::stream::Limits::default(), true),
        (v2(), false),
    ] {
        let (a, b) = tokio::io::duplex(4 * 1024 * 1024);
        let a = WebSocketStream::from_raw_socket(a, Role::Client, None).await;
        let mut peer = WebSocketStream::from_raw_socket(b, Role::Server, None).await;
        let stream =
            herdr_relay::stream::websocket_stream_with_limits(a, &[6; 16], limits).unwrap();
        let (mut reader, mut writer) = tokio::io::split(stream);
        let mut next = async || loop {
            if let Message::Binary(bytes) = peer.next().await.unwrap().unwrap() {
                let frame = Frame::decode(&bytes).unwrap();
                break (frame.kind, frame.offset);
            }
        };
        // Spend the whole data message burst with one-byte keystrokes.
        for offset in 0..300 {
            writer.write_all(b"k").await.unwrap();
            assert_eq!(next().await, (Kind::Data, offset));
        }
        let began = tokio::time::Instant::now();
        let frame = Frame {
            kind: Kind::Data,
            channel: [6; 16],
            offset: 0,
            payload: b"x",
        }
        .encode()
        .unwrap();
        peer.send(Message::Binary(frame.into())).await.unwrap();
        let mut byte = [0; 1];
        reader.read_exact(&mut byte).await.unwrap();
        let mut next = async || loop {
            if let Message::Binary(bytes) = peer.next().await.unwrap().unwrap() {
                let frame = Frame::decode(&bytes).unwrap();
                break (frame.kind, frame.offset);
            }
        };
        assert_eq!(next().await, (Kind::Credit, 1));
        // A shared, spent 150 message/s budget needs ~7 ms to refill one message.
        assert_eq!(
            began.elapsed() >= std::time::Duration::from_millis(5),
            delayed,
            "credit took {:?}",
            began.elapsed()
        );
    }
}
