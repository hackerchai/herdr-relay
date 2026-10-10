mod common;
use common::{Fixture, ORIGIN};
use herdr_relay::{
    control::{self, Purpose},
    direct, host, names, secure,
};

fn serve_control(
    fixture: &Fixture,
) -> (
    tokio::io::DuplexStream,
    tokio::task::JoinHandle<herdr_relay::Result<()>>,
) {
    let (host_side, client_side) = tokio::io::duplex(65536);
    let (profile, context, target) = (
        fixture.host(),
        fixture.context(Purpose::Control),
        fixture.target.clone(),
    );
    let task =
        tokio::spawn(async move { host::serve(host_side, &profile, &context, &[target]).await });
    (client_side, task)
}

#[tokio::test]
async fn listing_targets_teaches_each_side_the_others_hostname_and_the_direct_endpoint() {
    let fixture = Fixture::new();
    direct::advertise(Some(("66".repeat(16), 47120)));
    names::remember(
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        Some("Paired host"),
        None,
    )
    .unwrap();
    let (stream, task) = serve_control(&fixture);
    let stream = secure::connect(
        stream,
        &fixture.b,
        &fixture.a.certificate,
        &fixture.context(Purpose::Control),
    )
    .await
    .unwrap();
    let targets = host::list_targets(stream, &fixture.client(), ORIGIN, &fixture.a.id)
        .await
        .unwrap();
    assert_eq!(targets[0].id, fixture.target.id);
    task.await.unwrap().unwrap();
    let saved = names::remember(&fixture.client(), ORIGIN, &fixture.a.id, None, None).unwrap();
    assert_eq!(saved.name, names::hostname());
    assert_eq!(
        saved.alias,
        names::hostname(),
        "an untouched default alias follows the hostname"
    );
    let endpoint = direct::recall(&fixture.client(), ORIGIN, &fixture.a.id).unwrap();
    assert_eq!(
        (endpoint.listener_id.as_str(), endpoint.port),
        ("66".repeat(16).as_str(), 47120)
    );
    assert_eq!(
        names::peer_name(&fixture.host(), ORIGIN, &fixture.b.id).as_deref(),
        Some(names::hostname().as_str())
    );
}

#[tokio::test]
async fn a_custom_alias_survives_a_hostname_refresh() {
    let fixture = Fixture::new();
    names::remember(
        &fixture.client(),
        ORIGIN,
        &fixture.a.id,
        Some("old-name"),
        Some("my box"),
    )
    .unwrap();
    let (stream, task) = serve_control(&fixture);
    let stream = secure::connect(
        stream,
        &fixture.b,
        &fixture.a.certificate,
        &fixture.context(Purpose::Control),
    )
    .await
    .unwrap();
    host::list_targets(stream, &fixture.client(), ORIGIN, &fixture.a.id)
        .await
        .unwrap();
    task.await.unwrap().unwrap();
    let saved = names::remember(&fixture.client(), ORIGIN, &fixture.a.id, None, None).unwrap();
    assert_eq!(
        (saved.name.as_str(), saved.alias.as_str()),
        (names::hostname().as_str(), "my box")
    );
}

#[tokio::test]
async fn a_new_client_still_lists_targets_from_an_older_host() {
    let fixture = Fixture::new();
    let (host_side, client_side) = tokio::io::duplex(65536);
    let (a, b_cert, context) = (
        herdr_relay::identity::Identity::load(&fixture.host()).unwrap(),
        fixture.b.certificate.clone(),
        fixture.context(Purpose::Control),
    );
    let id = fixture.target.id.clone();
    // An older host: Targets, then Ack, then close; it never sends HostInfo.
    let old_host = tokio::spawn(async move {
        let mut stream = secure::accept(host_side, &a, &b_cert, &context)
            .await
            .unwrap();
        control::send(&mut stream, &serde_json::json!({"result":"targets","targets":[{"id":id,"name":"work","runtime_id":"77".repeat(16),"status":"online"}]}))
            .await
            .unwrap();
        let ack: serde_json::Value = control::receive(&mut stream).await.unwrap();
        assert_eq!(ack, serde_json::json!({}));
    });
    let stream = secure::connect(
        client_side,
        &fixture.b,
        &fixture.a.certificate,
        &fixture.context(Purpose::Control),
    )
    .await
    .unwrap();
    let targets = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        host::list_targets(stream, &fixture.client(), ORIGIN, &fixture.a.id),
    )
    .await
    .expect("waited for host info an older host never sends")
    .unwrap();
    assert_eq!(targets.len(), 1);
    old_host.await.unwrap();
}

#[tokio::test]
async fn an_older_client_still_lists_targets_from_a_new_host() {
    let fixture = Fixture::new();
    let (stream, task) = serve_control(&fixture);
    let mut stream = secure::connect(
        stream,
        &fixture.b,
        &fixture.a.certificate,
        &fixture.context(Purpose::Control),
    )
    .await
    .unwrap();
    // An older client: Targets, Ack, then close without reading further.
    let targets: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(targets["result"], "targets");
    control::send(&mut stream, &serde_json::json!({}))
        .await
        .unwrap();
    drop(stream);
    task.await.unwrap().unwrap();
}

#[tokio::test]
async fn only_name_derived_aliases_follow_a_hostname_change() {
    let fixture = Fixture::new();
    let profile = fixture.client();
    names::remember(
        &profile,
        ORIGIN,
        "h1",
        Some("MacBook"),
        Some("MacBook (work)"),
    )
    .unwrap();
    names::refresh_host(&profile, ORIGIN, "h1", "mbp-2").unwrap();
    assert_eq!(
        names::remember(&profile, ORIGIN, "h1", None, None)
            .unwrap()
            .alias,
        "MacBook (work)"
    );
    names::remember(&profile, ORIGIN, "h2", Some("box"), None).unwrap();
    names::remember(&profile, ORIGIN, "h3", Some("box"), None).unwrap(); // alias "box (2)"
    names::refresh_host(&profile, ORIGIN, "h3", "box-renamed").unwrap();
    assert_eq!(
        names::remember(&profile, ORIGIN, "h3", None, None)
            .unwrap()
            .alias,
        "box-renamed"
    );
}
