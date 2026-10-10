use herdr_relay::stream::websocket_stream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

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
