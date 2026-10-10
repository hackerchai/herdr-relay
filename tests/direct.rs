mod common;
use herdr_relay::direct::{Endpoint, LocalNet, advertisable, candidates};
use std::net::{IpAddr, SocketAddr};

fn ip(value: &str) -> IpAddr {
    value.parse().unwrap()
}
fn net(value: &str, prefix: u8) -> LocalNet {
    LocalNet {
        ip: ip(value),
        prefix,
    }
}
fn endpoint(addresses: &[&str]) -> Endpoint {
    Endpoint {
        listener_id: "44".repeat(16),
        port: 47120,
        addresses: addresses.iter().map(|a| ip(a)).collect(),
    }
}

#[test]
fn a_host_advertises_only_lan_and_tailnet_addresses() {
    for lan in [
        "192.168.1.20",
        "10.2.3.4",
        "172.20.0.5",
        "100.101.1.2",
        "fd7a:115c:a1e0::5",
        "fd00::1",
    ] {
        assert!(advertisable(ip(lan)), "{lan}");
    }
    for other in [
        "127.0.0.1",
        "::1",
        "169.254.1.1",
        "fe80::1",
        "8.8.8.8",
        "2606:4700::1",
        "0.0.0.0",
    ] {
        assert!(!advertisable(ip(other)), "{other}");
    }
}

#[test]
fn a_client_tries_hosts_on_its_own_subnet_before_tailnet_addresses() {
    let host = endpoint(&["100.101.1.2", "192.168.1.20", "10.9.9.9"]);
    let local = [net("192.168.1.5", 24), net("100.70.0.3", 32)];
    let expected: Vec<SocketAddr> = vec![
        "192.168.1.20:47120".parse().unwrap(),
        "100.101.1.2:47120".parse().unwrap(),
    ];
    assert_eq!(candidates(&host, &local), expected);
}

#[test]
fn a_client_off_the_hosts_networks_goes_straight_to_the_relay() {
    let host = endpoint(&["192.168.1.20", "100.101.1.2", "fd7a:115c:a1e0::5"]);
    assert!(candidates(&host, &[net("10.0.0.5", 8)]).is_empty());
    // Tailnet addresses need a tailnet interface on the client too.
    assert!(candidates(&host, &[net("192.168.7.2", 24)]).is_empty());
    let tailnet_v6 = candidates(&host, &[net("fd7a:115c:a1e0::9", 128)]);
    assert_eq!(
        tailnet_v6,
        vec!["[fd7a:115c:a1e0::5]:47120".parse::<SocketAddr>().unwrap()]
    );
}

#[test]
fn an_oversized_or_public_advertisement_is_bounded_and_filtered() {
    let many: Vec<String> = (1..=20).map(|i| format!("192.168.1.{i}")).collect();
    let mut addresses: Vec<&str> = many.iter().map(String::as_str).collect();
    addresses.insert(0, "8.8.8.8");
    let tried = candidates(&endpoint(&addresses), &[net("192.168.1.200", 24)]);
    assert_eq!(tried.len(), 8);
    assert!(tried.iter().all(|a| advertisable(a.ip())));
}

use common::{Fixture, ORIGIN};
use herdr_relay::{
    control::{self, Purpose},
    direct::{self, Listener},
};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn host(
    fixture: &Fixture,
) -> (
    Endpoint,
    tokio::sync::watch::Sender<u64>,
    tokio::task::JoinHandle<()>,
) {
    let listener = Listener::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let mut endpoint = listener.endpoint();
    endpoint.addresses = vec![ip("127.0.0.1")];
    let (revocations, watch) = tokio::sync::watch::channel(0);
    let registry = Arc::new(herdr_relay::targets::Registry::open(&fixture.host()).unwrap());
    let profile = fixture.host();
    let task = tokio::spawn(async move {
        let _ = listener.serve(&profile, ORIGIN, registry, watch).await;
    });
    (endpoint, revocations, task)
}
fn addresses(endpoint: &Endpoint) -> Vec<SocketAddr> {
    vec![SocketAddr::new(endpoint.addresses[0], endpoint.port)]
}

#[tokio::test]
async fn a_paired_client_lists_targets_directly_on_the_lan() {
    let fixture = Fixture::new();
    let (endpoint, _revocations, _task) = host(&fixture).await;
    let connected = direct::connect_to(
        &addresses(&endpoint),
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        &endpoint.listener_id,
        Purpose::Control,
        None,
    )
    .await
    .unwrap();
    assert_eq!(connected.address, addresses(&endpoint)[0]);
    let mut stream = connected.stream;
    let targets: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(targets["result"], "targets");
    assert_eq!(targets["targets"][0]["id"], fixture.target.id);
}

#[tokio::test]
async fn a_direct_attach_reaches_the_native_session_and_holds_the_single_client_reservation() {
    let fixture = Fixture::new();
    let (endpoint, _revocations, _task) = host(&fixture).await;
    let connected = direct::connect_to(
        &addresses(&endpoint),
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        &endpoint.listener_id,
        Purpose::Attach,
        Some(&fixture.target.id),
    )
    .await
    .unwrap();
    let mut stream = connected.stream;
    let ready: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(ready["result"], "ready");
    // A second client is refused while the first one holds the target.
    let mut second = direct::connect_to(
        &addresses(&endpoint),
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        &endpoint.listener_id,
        Purpose::Attach,
        Some(&fixture.target.id),
    )
    .await
    .unwrap()
    .stream;
    let busy: serde_json::Value = control::receive(&mut second).await.unwrap();
    assert_eq!(busy["code"], "TARGET_BUSY");
    control::send(
        &mut stream,
        &serde_json::json!({"runtime_id":ready["runtime_id"]}),
    )
    .await
    .unwrap();
    let started: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(started["result"], "started");
    let (mut native, _) = fixture.native.accept().await.unwrap();
    stream.write_all(b"hello").await.unwrap();
    stream.flush().await.unwrap();
    let mut bytes = [0; 5];
    native.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"hello");
}

#[tokio::test]
async fn an_unpaired_device_or_a_stale_listener_generation_is_refused() {
    let fixture = Fixture::new();
    let (endpoint, _revocations, _task) = host(&fixture).await;
    let stranger = fixture.temp.path().join("stranger");
    herdr_relay::identity::Identity::initialize(&stranger).unwrap();
    // A stranger has no grant for this host, so it cannot even build a request.
    assert!(
        direct::connect_to(
            &addresses(&endpoint),
            &stranger,
            ORIGIN,
            &fixture.a.id,
            &endpoint.listener_id,
            Purpose::Control,
            None
        )
        .await
        .is_err()
    );
    let stale = direct::connect_to(
        &addresses(&endpoint),
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        &"55".repeat(16),
        Purpose::Control,
        None,
    )
    .await;
    assert!(stale.is_err());
}

#[tokio::test]
async fn revoking_a_device_ends_its_direct_session() {
    let fixture = Fixture::new();
    let (endpoint, revocations, _task) = host(&fixture).await;
    let mut stream = direct::connect_to(
        &addresses(&endpoint),
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        &endpoint.listener_id,
        Purpose::Attach,
        Some(&fixture.target.id),
    )
    .await
    .unwrap()
    .stream;
    let ready: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(ready["result"], "ready");
    herdr_relay::pairing::Store::open(&fixture.host(), ORIGIN)
        .unwrap()
        .revoke_peer(&fixture.b.id)
        .unwrap();
    revocations.send_modify(|n| *n += 1);
    let mut byte = [0; 1];
    let ended = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte)).await;
    assert!(
        matches!(ended, Ok(Ok(0)) | Ok(Err(_))),
        "revoked session stayed open"
    );
}

#[tokio::test]
async fn the_listener_refuses_a_device_that_no_grant_pins_even_if_it_knows_the_host() {
    let fixture = Fixture::new();
    let (endpoint, _revocations, _task) = host(&fixture).await;
    let stranger =
        herdr_relay::identity::Identity::initialize(&fixture.temp.path().join("s")).unwrap();
    let mut context = fixture.context(Purpose::Control);
    context.connector = stranger.id.clone();
    context.boot_id = endpoint.listener_id.clone();
    let tcp = tokio::net::TcpStream::connect(addresses(&endpoint)[0])
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        herdr_relay::secure::connect(tcp, &stranger, &fixture.a.certificate, &context),
    )
    .await
    .unwrap();
    assert!(result.is_err());
}

mod fallback {
    use herdr_relay::{
        Error,
        direct::{Route, with_fallback},
    };
    use std::{
        net::SocketAddr,
        sync::atomic::{AtomicBool, Ordering},
    };
    fn lan() -> SocketAddr {
        "192.168.1.20:47120".parse().unwrap()
    }

    #[tokio::test]
    async fn a_reachable_host_is_used_directly_and_the_relay_is_never_opened() {
        let relay_opened = AtomicBool::new(false);
        let (result, route, failure) =
            with_fallback(Some(async { Ok(("direct", lan())) }), || async {
                relay_opened.store(true, Ordering::SeqCst);
                Ok("relay")
            })
            .await;
        assert_eq!(result.unwrap(), "direct");
        assert_eq!(route, Route::Direct(lan()));
        assert!(failure.is_none());
        assert!(!relay_opened.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn a_failed_direct_probe_falls_back_to_the_relay_and_reports_why() {
        let (result, route, failure) = with_fallback(
            Some(async { Err::<(&str, SocketAddr), _>(Error::new("DIRECT_UNREACHABLE", "x")) }),
            || async { Ok("relay") },
        )
        .await;
        assert_eq!(result.unwrap(), "relay");
        assert_eq!(route, Route::Relay);
        assert_eq!(failure.unwrap().code, "DIRECT_UNREACHABLE");
    }

    #[tokio::test]
    async fn without_a_direct_endpoint_the_relay_is_used() {
        let none: Option<std::future::Ready<herdr_relay::Result<(&str, SocketAddr)>>> = None;
        let (result, route, failure) = with_fallback(none, || async { Ok("relay") }).await;
        assert_eq!((result.unwrap(), route), ("relay", Route::Relay));
        assert!(failure.is_none());
    }
}

#[test]
fn the_direct_listener_is_on_by_default_and_configurable_through_the_environment() {
    use herdr_relay::direct::{DEFAULT_PORT, listener_port};
    assert_eq!(listener_port(None, None), Some(DEFAULT_PORT));
    assert_eq!(listener_port(Some("on"), Some("50001")), Some(50001));
    assert_eq!(listener_port(None, Some("0")), Some(0));
    for off in ["off", "0", "false", "no"] {
        assert_eq!(listener_port(Some(off), None), None, "{off}");
    }
    // An invalid port keeps the safe default instead of disabling or failing.
    assert_eq!(listener_port(None, Some("http")), Some(DEFAULT_PORT));
}

#[tokio::test]
async fn a_reachable_first_candidate_is_used_without_opening_the_others() {
    let fixture = Fixture::new();
    let (endpoint, _revocations, _task) = host(&fixture).await;
    // A second listener stands in for another interface of the same host.
    let spare = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let spare_address = spare.local_addr().unwrap();
    let connected = direct::connect_to(
        &[addresses(&endpoint)[0], spare_address],
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        &endpoint.listener_id,
        Purpose::Control,
        None,
    )
    .await
    .unwrap();
    assert_eq!(connected.address, addresses(&endpoint)[0]);
    assert!(
        tokio::time::timeout(Duration::from_millis(400), spare.accept())
            .await
            .is_err(),
        "a later candidate was opened although the first one answered"
    );
}

#[tokio::test]
async fn a_candidate_that_accepts_tcp_but_is_not_the_host_does_not_end_the_direct_attempt() {
    let fixture = Fixture::new();
    let (endpoint, _revocations, _task) = host(&fixture).await;
    // Some other service answers TCP on the first address and closes.
    let impostor = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let impostor_address = impostor.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((socket, _)) = impostor.accept().await {
            drop(socket);
        }
    });
    let connected = direct::connect_to(
        &[impostor_address, addresses(&endpoint)[0]],
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        &endpoint.listener_id,
        Purpose::Control,
        None,
    )
    .await
    .unwrap();
    assert_eq!(connected.address, addresses(&endpoint)[0]);
}
