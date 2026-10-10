mod common;
use common::Fixture;
use herdr_relay::{
    Error,
    control::{self, Purpose},
    host, secure,
};
use std::time::Duration;

/// The relay's `authenticated` acknowledgement runs beside the endpoint
/// exchange instead of delaying Ready/Targets by its own round trips.
#[tokio::test]
async fn ready_is_sent_while_the_relay_acknowledgement_is_still_pending() {
    let fixture = Fixture::new();
    let context = fixture.context(Purpose::Attach);
    let (gate, wait) = tokio::sync::oneshot::channel::<herdr_relay::Result<()>>();
    let (host_side, client_side) = tokio::io::duplex(65536);
    let (profile, expected, target) = (fixture.host(), context.clone(), fixture.target.clone());
    let task = tokio::spawn(async move {
        host::serve_after_auth(host_side, &profile, &expected, &[target], || async {
            wait.await.unwrap()
        })
        .await
    });
    let mut stream = secure::connect(client_side, &fixture.b, &fixture.a.certificate, &context)
        .await
        .unwrap();
    let ready: serde_json::Value =
        tokio::time::timeout(Duration::from_secs(2), control::receive(&mut stream))
            .await
            .expect("Ready waited for the relay acknowledgement")
            .unwrap();
    assert_eq!(ready["result"], "ready");
    control::send(
        &mut stream,
        &serde_json::json!({"runtime_id":ready["runtime_id"]}),
    )
    .await
    .unwrap();
    // The native stream must not open before the relay accepted the channel.
    assert!(
        tokio::time::timeout(Duration::from_millis(50), fixture.native.accept())
            .await
            .is_err()
    );
    gate.send(Ok(())).unwrap();
    let started: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(started["result"], "started");
    drop(stream);
    let _ = task.await;
}

#[tokio::test]
async fn a_failed_relay_acknowledgement_rejects_the_attach_before_native_start() {
    let fixture = Fixture::new();
    let context = fixture.context(Purpose::Attach);
    let (host_side, client_side) = tokio::io::duplex(65536);
    let (profile, expected, target) = (fixture.host(), context.clone(), fixture.target.clone());
    let task = tokio::spawn(async move {
        host::serve_after_auth(host_side, &profile, &expected, &[target], || async {
            Err(Error::new("CHANNEL_EXPIRED", "relay dropped the channel"))
        })
        .await
    });
    let mut stream = secure::connect(client_side, &fixture.b, &fixture.a.certificate, &context)
        .await
        .unwrap();
    // The rejection may replace Ready or follow Start; it never becomes Started.
    let mut response: serde_json::Value = control::receive(&mut stream).await.unwrap();
    if response["result"] == "ready" {
        control::send(
            &mut stream,
            &serde_json::json!({"runtime_id":response["runtime_id"]}),
        )
        .await
        .unwrap();
        response = control::receive(&mut stream).await.unwrap();
    }
    assert_eq!(response["result"], "rejected");
    assert_eq!(response["code"], "CHANNEL_EXPIRED");
    control::send(&mut stream, &serde_json::json!({}))
        .await
        .unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, "CHANNEL_EXPIRED");
    assert!(
        tokio::time::timeout(Duration::from_millis(50), fixture.native.accept())
            .await
            .is_err()
    );
    assert_eq!(fixture.target.status().status, "online");
}

#[tokio::test]
async fn targets_are_listed_while_the_relay_acknowledgement_is_still_pending() {
    let fixture = Fixture::new();
    let context = fixture.context(Purpose::Control);
    let (gate, wait) = tokio::sync::oneshot::channel::<herdr_relay::Result<()>>();
    let (host_side, client_side) = tokio::io::duplex(65536);
    let (profile, expected, target) = (fixture.host(), context.clone(), fixture.target.clone());
    let task = tokio::spawn(async move {
        host::serve_after_auth(host_side, &profile, &expected, &[target], || async {
            wait.await.unwrap()
        })
        .await
    });
    let mut stream = secure::connect(client_side, &fixture.b, &fixture.a.certificate, &context)
        .await
        .unwrap();
    let targets: serde_json::Value =
        tokio::time::timeout(Duration::from_secs(2), control::receive(&mut stream))
            .await
            .expect("Targets waited for the relay acknowledgement")
            .unwrap();
    assert_eq!(targets["result"], "targets");
    gate.send(Ok(())).unwrap();
    drop(stream);
    let _ = task.await;
}

#[tokio::test]
async fn a_completed_listing_is_not_failed_by_a_late_relay_acknowledgement() {
    let fixture = Fixture::new();
    let context = fixture.context(Purpose::Control);
    let (host_side, client_side) = tokio::io::duplex(65536);
    let (profile, expected, target) = (fixture.host(), context.clone(), fixture.target.clone());
    let task = tokio::spawn(async move {
        host::serve_after_auth(host_side, &profile, &expected, &[target], || async {
            // The client already closed the channel, so the relay refuses it.
            tokio::time::sleep(Duration::from_millis(200)).await;
            Err(Error::new("NOT_AUTHORIZED", "channel already ended"))
        })
        .await
    });
    let stream = secure::connect(client_side, &fixture.b, &fixture.a.certificate, &context)
        .await
        .unwrap();
    let targets = host::list_targets(stream, &fixture.client(), common::ORIGIN, &fixture.a.id)
        .await
        .unwrap();
    assert_eq!(targets.len(), 1);
    task.await
        .unwrap()
        .expect("a delivered listing is a success");
}
