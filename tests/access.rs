use herdr_relay::access::Challenge;

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
