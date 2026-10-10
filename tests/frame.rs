use herdr_relay::frame::{Frame, Kind};
#[test]
fn shared_frame_fixtures_preserve_exact_offsets_and_enforce_envelope_bounds() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../protocol/fixtures/frames-v1.json")).unwrap();
    for case in fixture["valid"].as_array().unwrap() {
        let bytes = hex::decode(case["hex"].as_str().unwrap()).unwrap();
        let frame = Frame::decode(&bytes).unwrap();
        assert_eq!(frame.offset.to_string(), case["offset"].as_str().unwrap());
        assert_eq!(
            hex::encode(frame.channel),
            case["channel"].as_str().unwrap()
        );
        assert_eq!(
            hex::encode(frame.payload),
            case["payload"].as_str().unwrap()
        );
        assert_eq!(
            if frame.kind == Kind::Data { 1 } else { 2 },
            case["kind"].as_u64().unwrap()
        );
        assert_eq!(frame.encode().unwrap(), bytes);
    }
    for case in fixture["invalid"].as_array().unwrap() {
        assert!(
            Frame::decode(&hex::decode(case["hex"].as_str().unwrap()).unwrap()).is_err(),
            "{}",
            case["name"]
        );
    }
}
