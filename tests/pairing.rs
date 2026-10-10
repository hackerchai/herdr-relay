use herdr_relay::{identity::Identity, pairing::Store};
use std::os::unix::fs::PermissionsExt;

fn device() -> (tempfile::TempDir, Identity) {
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let identity = Identity::initialize(dir.path()).unwrap();
    (dir, identity)
}

#[test]
fn an_invitation_grants_only_the_authenticated_connector_and_survives_restart() {
    let (a_dir, a) = device();
    let (b_dir, b) = device();
    let store = Store::open(a_dir.path(), "https://relay.example").unwrap();
    let invitation = store.invite(1000).unwrap();
    let request = Store::open(b_dir.path(), &invitation.origin)
        .unwrap()
        .prepare(&invitation, 1001)
        .unwrap();
    let receipt = store.redeem(&b.certificate, &request, 1001).unwrap();
    let restarted = Store::open(a_dir.path(), "https://relay.example").unwrap();
    let grants = restarted.grants().unwrap();
    assert_eq!(grants.allowed_here.len(), 1);
    assert_eq!(grants.allowed_here[0].device_id, b.id);
    assert!(grants.accessible_hosts.is_empty());
    let connector = Store::open(b_dir.path(), &invitation.origin).unwrap();
    connector
        .complete(&invitation, &request, &receipt, 1002)
        .unwrap();
    let grants = connector.grants().unwrap();
    assert!(grants.allowed_here.is_empty());
    assert_eq!(grants.accessible_hosts.len(), 1);
    assert_eq!(grants.accessible_hosts[0].device_id, a.id);
}

#[test]
fn racing_connectors_cannot_consume_one_invitation_twice_and_only_the_winner_can_retry() {
    let (a_dir, _) = device();
    let (b_dir, b) = device();
    let (c_dir, c) = device();
    let origin = "https://relay.example";
    let store = Store::open(a_dir.path(), origin).unwrap();
    let invitation = store.invite(1000).unwrap();
    let b_request = Store::open(b_dir.path(), origin)
        .unwrap()
        .prepare(&invitation, 1001)
        .unwrap();
    let c_request = Store::open(c_dir.path(), origin)
        .unwrap()
        .prepare(&invitation, 1001)
        .unwrap();
    let barrier = std::sync::Barrier::new(2);
    let (b_result, c_result) = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            Store::open(a_dir.path(), origin)
                .unwrap()
                .redeem(&b.certificate, &b_request, 1001)
        });
        let second = scope.spawn(|| {
            barrier.wait();
            Store::open(a_dir.path(), origin)
                .unwrap()
                .redeem(&c.certificate, &c_request, 1001)
        });
        (first.join().unwrap(), second.join().unwrap())
    });
    assert_ne!(b_result.is_ok(), c_result.is_ok());
    assert_eq!(store.grants().unwrap().allowed_here.len(), 1);
    let (winner, request, receipt) = if let Ok(receipt) = b_result {
        (&b, b_request, receipt)
    } else {
        (&c, c_request, c_result.unwrap())
    };
    let restarted = Store::open(a_dir.path(), origin).unwrap();
    // Even after the original ten-minute invitation expiry, the original request
    // can recover its receipt within the bounded result-retention interval.
    assert_eq!(
        restarted
            .redeem(&winner.certificate, &request, 2000)
            .unwrap(),
        receipt
    );
    let mut different = request.clone();
    different.request_id = "ff".repeat(16);
    assert_eq!(
        restarted
            .redeem(&winner.certificate, &different, 2000)
            .unwrap_err()
            .code,
        "PAIR_INVALID"
    );
    different = request.clone();
    different.secret = "00".repeat(32);
    assert_eq!(
        restarted
            .redeem(&winner.certificate, &different, 2000)
            .unwrap_err()
            .code,
        "PAIR_INVALID"
    );
    assert_eq!(
        restarted
            .redeem(&winner.certificate, &request, 87401)
            .unwrap_err()
            .code,
        "PAIR_INVALID"
    );
}

#[test]
fn invalid_expired_revoked_and_failed_writes_do_not_create_grants() {
    let (a_dir, _) = device();
    let (b_dir, b) = device();
    let origin = "https://relay.example";
    let host = Store::open(a_dir.path(), origin).unwrap();
    let connector = Store::open(b_dir.path(), origin).unwrap();
    let invitation = host.invite(1000).unwrap();
    let request = connector.prepare(&invitation, 1001).unwrap();
    let mut incorrect = request.clone();
    incorrect.secret = "00".repeat(32);
    assert_eq!(
        host.redeem(&b.certificate, &incorrect, 1001)
            .unwrap_err()
            .code,
        "PAIR_INVALID"
    );
    assert_eq!(
        host.redeem(&b.certificate, &request, 1600)
            .unwrap_err()
            .code,
        "PAIR_INVALID"
    );
    host.revoke_invitation(&invitation.invitation_id).unwrap();
    assert_eq!(
        host.redeem(&b.certificate, &request, 1002)
            .unwrap_err()
            .code,
        "PAIR_INVALID"
    );
    let invitation = host.invite(2000).unwrap();
    let request = connector.prepare(&invitation, 2001).unwrap();
    std::fs::set_permissions(a_dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
    let failed = host.redeem(&b.certificate, &request, 2001);
    std::fs::set_permissions(a_dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(failed.unwrap_err().code, "PAIR_STORAGE");
    assert!(host.grants().unwrap().allowed_here.is_empty());
    host.redeem(&b.certificate, &request, 2002).unwrap();
    assert_eq!(host.grants().unwrap().allowed_here.len(), 1);
}

#[tokio::test]
async fn unpaired_connector_can_redeem_over_tls_but_wrong_secret_never_grants_access() {
    use herdr_relay::pairing;
    let (a_dir, _) = device();
    let (b_dir, _) = device();
    let origin = "https://relay.example";
    let host = Store::open(a_dir.path(), origin).unwrap();
    let connector = Store::open(b_dir.path(), origin).unwrap();
    let invitation = host.invite(pairing::now()).unwrap();
    let (a, b) = tokio::io::duplex(4096);
    let channel = "11".repeat(16);
    let boot = "22".repeat(16);
    let (served, accepted) = tokio::join!(
        pairing::serve(a, &host, &channel, &boot),
        pairing::accept(b, &connector, &invitation, &channel, &boot)
    );
    served.unwrap();
    accepted.unwrap();
    assert_eq!(host.grants().unwrap().allowed_here.len(), 1);
    assert_eq!(connector.grants().unwrap().accessible_hosts.len(), 1);
    let mut invalid = host.invite(pairing::now()).unwrap();
    invalid.secret = "ff".repeat(32);
    let (a, b) = tokio::io::duplex(4096);
    let (served, accepted) = tokio::join!(
        pairing::serve(a, &host, &channel, &boot),
        pairing::accept(b, &connector, &invalid, &channel, &boot)
    );
    assert_eq!(served.unwrap_err().code, "PAIR_INVALID");
    assert_eq!(accepted.unwrap_err().code, "PAIR_INVALID");
    assert_eq!(host.grants().unwrap().allowed_here.len(), 1);
}

#[tokio::test]
async fn publisher_keeps_the_channel_until_the_connector_acknowledges_the_result() {
    use herdr_relay::{
        control::{self, Context, Purpose},
        pairing, secure,
    };
    use std::time::Duration;
    let (a_dir, a) = device();
    let (b_dir, b) = device();
    let host = Store::open(a_dir.path(), "https://relay.example").unwrap();
    let invitation = host.invite(pairing::now()).unwrap();
    let connector = Store::open(b_dir.path(), "https://relay.example").unwrap();
    let request = connector.prepare(&invitation, pairing::now()).unwrap();
    let context = Context {
        version: 1,
        origin: invitation.origin.clone(),
        publisher: a.id.clone(),
        connector: b.id.clone(),
        purpose: Purpose::Pair,
        grant_version: None,
        target: None,
        channel_id: "11".repeat(16),
        boot_id: "22".repeat(16),
    };
    let (c, s) = tokio::io::duplex(4096);
    let channel = context.channel_id.clone();
    let boot = context.boot_id.clone();
    let mut server = tokio::spawn(async move { pairing::serve(s, &host, &channel, &boot).await });
    let mut stream = secure::connect(c, &b, &a.certificate, &context)
        .await
        .unwrap();
    control::send(&mut stream, &request).await.unwrap();
    let result: serde_json::Value = control::receive(&mut stream).await.unwrap();
    assert_eq!(result["result"], "granted");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut server)
            .await
            .is_err(),
        "publisher closed before result acknowledgement"
    );
    control::send(
        &mut stream,
        &serde_json::json!({"request_id":request.request_id}),
    )
    .await
    .unwrap();
    server.await.unwrap().unwrap();
}

#[test]
fn revocation_survives_restart_and_old_receipts_cannot_restore_a_grant() {
    let (a_dir, a) = device();
    let (b_dir, b) = device();
    let (c_dir, c) = device();
    let origin = "https://relay.example";
    let host = Store::open(a_dir.path(), origin).unwrap();
    let reverse = Store::open(b_dir.path(), origin).unwrap();
    let pair = |store: &Store, peer: &herdr_relay::identity::Identity| {
        let invitation = store.invite(100).unwrap();
        let request = herdr_relay::pairing::RedeemRequest {
            invitation_id: invitation.invitation_id,
            secret: invitation.secret,
            request_id: herdr_relay::control::random_id().unwrap(),
        };
        let receipt = store.redeem(&peer.certificate, &request, 100).unwrap();
        (request, receipt)
    };
    let (old, receipt) = pair(&host, &b);
    pair(&host, &c);
    pair(&reverse, &a);
    host.revoke_peer(&b.id).unwrap();
    let host = Store::open(a_dir.path(), origin).unwrap();
    assert_eq!(
        host.grants()
            .unwrap()
            .allowed_here
            .iter()
            .map(|g| &g.device_id)
            .collect::<Vec<_>>(),
        vec![&c.id]
    );
    assert_eq!(
        host.redeem(&b.certificate, &old, 101).unwrap_err().code,
        "PAIR_INVALID"
    );
    assert_eq!(reverse.grants().unwrap().allowed_here[0].device_id, a.id);
    let (_, new_receipt) = pair(&host, &b);
    assert!(new_receipt.grant_version > receipt.grant_version);
    assert_eq!(
        host.redeem(&b.certificate, &old, 102).unwrap_err().code,
        "PAIR_INVALID"
    );
    drop(c_dir);
}

#[test]
fn explicit_origin_switch_requires_stopped_service_and_discards_old_trust_only() {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let (a_dir, a) = device();
    let (b_dir, b) = device();
    let old = "https://old.example";
    let new = "https://new.example";
    let publisher = Store::open(a_dir.path(), old).unwrap();
    let connector = Store::open(b_dir.path(), old).unwrap();
    let invitation = publisher.invite(1000).unwrap();
    let request = connector.prepare(&invitation, 1001).unwrap();
    let receipt = publisher.redeem(&b.certificate, &request, 1001).unwrap();
    connector
        .complete(&invitation, &request, &receipt, 1001)
        .unwrap();
    let reverse = connector.invite(1001).unwrap();
    let pending = publisher.prepare(&reverse, 1001).unwrap();
    let reverse_receipt = connector.redeem(&a.certificate, &pending, 1001).unwrap();
    publisher
        .complete(&reverse, &pending, &reverse_receipt, 1001)
        .unwrap();
    let socket = a_dir.path().join("native.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
    let targets = herdr_relay::targets::Store::open(a_dir.path()).unwrap();
    let target = targets
        .add(
            "Work".into(),
            herdr_relay::targets::Locator::Socket { path: socket },
        )
        .unwrap();
    let key_before = std::fs::read(a_dir.path().join("identity/device.pk8")).unwrap();
    let state_before = std::fs::read(a_dir.path().join("pairing.json")).unwrap();
    Store::switch_origin(a_dir.path(), old, old).unwrap();
    assert_eq!(
        std::fs::read(a_dir.path().join("pairing.json")).unwrap(),
        state_before
    );
    let daemon = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(a_dir.path().join("daemon.lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(daemon.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    assert_eq!(
        Store::switch_origin(a_dir.path(), old, new)
            .unwrap_err()
            .code,
        "SERVICE_BUSY"
    );
    drop(daemon);
    assert!(Store::switch_origin(a_dir.path(), "https://wrong.example", new).is_err());
    assert_eq!(
        std::fs::read(a_dir.path().join("pairing.json")).unwrap(),
        state_before
    );
    Store::switch_origin(a_dir.path(), old, new).unwrap();
    Store::switch_origin(b_dir.path(), old, new).unwrap();
    assert_eq!(publisher.grants().unwrap_err().code, "PROFILE_MISMATCH");
    let publisher = Store::open(a_dir.path(), new).unwrap();
    let connector = Store::open(b_dir.path(), new).unwrap();
    for store in [&publisher, &connector] {
        let grants = store.grants().unwrap();
        assert!(grants.allowed_here.is_empty() && grants.accessible_hosts.is_empty());
    }
    assert!(connector.prepare(&invitation, 1002).is_err());
    assert!(publisher.redeem(&b.certificate, &request, 1002).is_err());
    assert_eq!(Identity::load(a_dir.path()).unwrap().id, a.id);
    assert_eq!(
        std::fs::read(a_dir.path().join("identity/device.pk8")).unwrap(),
        key_before
    );
    assert_eq!(targets.list().unwrap()[0].id, target.id);
    let invitation = publisher.invite(1003).unwrap();
    let request = connector.prepare(&invitation, 1003).unwrap();
    assert!(
        publisher
            .redeem(&b.certificate, &request, 1003)
            .unwrap()
            .grant_version
            > receipt.grant_version
    );
}
