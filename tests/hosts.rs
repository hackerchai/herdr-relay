use herdr_relay::{hosts, identity::Identity, names, pairing::Store};
use std::os::unix::fs::PermissionsExt;

const ORIGIN: &str = "https://relay.example";

fn device() -> (tempfile::TempDir, Identity) {
    let dir = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let identity = Identity::initialize(dir.path()).unwrap();
    (dir, identity)
}

/// Pair `client` with `host` the way the relay flow does, naming the host.
fn pair(host: &std::path::Path, client: &(tempfile::TempDir, Identity), name: &str, now: u64) {
    let store = Store::open(host, ORIGIN).unwrap();
    let invitation = store.invite(now).unwrap();
    let connector = Store::open(client.0.path(), ORIGIN).unwrap();
    let request = connector.prepare(&invitation, now + 1).unwrap();
    let receipt = store
        .redeem(&client.1.certificate, &request, now + 1)
        .unwrap();
    connector
        .complete(&invitation, &request, &receipt, now + 2)
        .unwrap();
    names::remember(
        client.0.path(),
        ORIGIN,
        &invitation.publisher,
        Some(name),
        None,
    )
    .unwrap();
}

fn accessible(client: &std::path::Path) -> Vec<String> {
    Store::open(client, ORIGIN)
        .unwrap()
        .grants()
        .unwrap()
        .accessible_hosts
        .into_iter()
        .map(|g| g.device_id)
        .collect()
}

#[test]
fn a_host_can_be_removed_by_alias_or_device_id_and_paired_again() {
    let (ubuntu_dir, ubuntu) = device();
    let (mac_dir, mac) = device();
    let client = device();
    pair(ubuntu_dir.path(), &client, "ubuntu", 1000);
    pair(mac_dir.path(), &client, "mac", 2000);
    assert_eq!(hosts::list(client.0.path(), ORIGIN).unwrap().len(), 2);

    let removed = hosts::remove(client.0.path(), ORIGIN, "ubuntu").unwrap();
    assert_eq!(removed.identity, ubuntu.id);
    assert_eq!(accessible(client.0.path()), vec![mac.id.clone()]);
    let listed = hosts::list(client.0.path(), ORIGIN).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].alias, "mac");
    assert_eq!(
        hosts::remove(client.0.path(), ORIGIN, "ubuntu")
            .unwrap_err()
            .code,
        "HOST_UNKNOWN"
    );

    hosts::remove(client.0.path(), ORIGIN, &mac.id).unwrap();
    assert!(accessible(client.0.path()).is_empty());
    assert!(hosts::list(client.0.path(), ORIGIN).unwrap().is_empty());

    // Removing is local: a fresh invitation pairs the host again.
    pair(ubuntu_dir.path(), &client, "ubuntu", 3000);
    assert_eq!(accessible(client.0.path()), vec![ubuntu.id]);
}

#[test]
fn an_unknown_host_is_reported_and_nothing_changes() {
    let (host_dir, _) = device();
    let client = device();
    pair(host_dir.path(), &client, "ubuntu", 1000);
    for selector in ["", "nope", "ubunt"] {
        assert_eq!(
            hosts::remove(client.0.path(), ORIGIN, selector)
                .unwrap_err()
                .code,
            "HOST_UNKNOWN",
            "{selector:?}"
        );
    }
    assert_eq!(accessible(client.0.path()).len(), 1);
}
