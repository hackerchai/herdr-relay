use herdr_relay::transport::bridge;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn opaque_bytes_cross_in_both_directions_and_disconnect_ends_the_channel() {
    tokio::time::timeout(Duration::from_secs(2), async {
        let (mut native_client, a) = tokio::io::duplex(1024);
        let (b, mut native_server) = tokio::io::duplex(1024);
        let bridge = tokio::spawn(bridge(a, b));
        native_client
            .write_all(b"\x00\xffhello\xe4\xb8\xad\xe6\x96\x87")
            .await
            .unwrap();
        let mut input = [0; 13];
        native_server.read_exact(&mut input).await.unwrap();
        assert_eq!(&input, b"\x00\xffhello\xe4\xb8\xad\xe6\x96\x87");
        // An opaque native frame larger than any Relay chunk must stream.
        let writer = tokio::spawn(async move {
            for _ in 0..128 {
                native_server.write_all(&[0xa7; 4096]).await.unwrap();
            }
            native_server
        });
        let mut output = vec![0; 512 * 1024];
        native_client.read_exact(&mut output).await.unwrap();
        assert!(output.iter().all(|&b| b == 0xa7));
        let _server = writer.await.unwrap();
        drop(native_client);
        bridge.await.unwrap().unwrap();
    })
    .await
    .expect("bridge must not keep a half-open channel alive");
}

#[tokio::test]
async fn native_handshake_cannot_wait_forever_for_a_silent_server() {
    let (_client, a) = tokio::io::duplex(64);
    let (b, _server) = tokio::io::duplex(64);
    let result = tokio::time::timeout(
        Duration::from_millis(500),
        herdr_relay::transport::bridge_with_handshake(a, b, Duration::from_millis(30)),
    )
    .await;
    assert_eq!(
        result
            .expect("native hello deadline must be enforced")
            .unwrap_err()
            .code,
        "HANDSHAKE_TIMEOUT"
    );
}

#[tokio::test]
async fn native_detach_and_remote_eof_have_distinct_recovery_outcomes() {
    use herdr_relay::transport::{NativeEnd, bridge_native};
    for remote_closes in [false, true] {
        let (mut client, a) = tokio::io::duplex(64);
        let (b, mut server) = tokio::io::duplex(64);
        let task = tokio::spawn(bridge_native(a, b, Duration::from_secs(1)));
        server.write_all(b"hello").await.unwrap();
        let mut hello = [0; 5];
        client.read_exact(&mut hello).await.unwrap();
        if remote_closes {
            drop(server);
        } else {
            drop(client);
        }
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            if remote_closes {
                NativeEnd::Server
            } else {
                NativeEnd::Client
            }
        );
    }
}
