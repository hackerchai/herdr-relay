use herdr_relay::{
    access::{Challenge, public_key},
    identity::Identity,
    retry::Backoff,
};

#[test]
fn rust_uses_the_same_access_signature_bytes_as_the_worker() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../protocol/fixtures/access-v1.json")).unwrap();
    let challenge: Challenge = serde_json::from_value(fixture["challenge"].clone()).unwrap();
    assert_eq!(
        challenge.canonical().unwrap(),
        fixture["canonical"].as_str().unwrap()
    );
    let mut bad = challenge.clone();
    bad.origin = "https://relay.example/".into();
    assert!(bad.canonical().is_err());
    bad = challenge.clone();
    bad.expires_at = "01800000060".into();
    assert!(bad.canonical().is_err());
    bad = challenge;
    bad.version = 2;
    assert!(bad.canonical().is_err());
}

#[test]
fn modest_clock_skew_does_not_misclassify_a_fresh_challenge_as_an_identity_failure() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let identity = Identity::initialize(root.path()).unwrap();
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../protocol/fixtures/access-v1.json")).unwrap();
    let mut challenge: Challenge = serde_json::from_value(fixture["challenge"].clone()).unwrap();
    challenge.device_id = identity.id.clone();
    challenge.public_key = public_key(&identity).unwrap();
    for client_time in [1799999999, 1800000000, 1800000120] {
        let proof = challenge
            .clone()
            .sign(&identity, client_time)
            .expect("small clock offsets are normal");
        assert_eq!(
            proof.challenge.expires_at, "1800000060",
            "signing cannot extend server expiry"
        );
    }
    for client_time in [1799999000, 1800001000] {
        let error = challenge
            .clone()
            .sign(&identity, client_time)
            .err()
            .unwrap();
        assert_eq!(error.code, "CLOCK_SKEW");
        assert!(Backoff::default().next(&error, 0).unwrap().as_secs() >= 60);
    }
    challenge.public_key = "0".repeat(64);
    assert_eq!(
        challenge.sign(&identity, 1800000000).err().unwrap().code,
        "ACCESS_INVALID"
    );
}
