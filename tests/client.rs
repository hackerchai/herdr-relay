use herdr_relay::client::check_version;
use std::os::unix::fs::PermissionsExt;

#[tokio::test]
async fn known_bounded_native_build_is_compatible_but_unknown_builds_are_rejected() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("herdr");
    std::fs::write(
        &binary,
        "#!/bin/sh\nprintf '%s\\n' 'herdr 0.9.3-custom.relay-control-bound-1'\n",
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    check_version(&binary).await.unwrap();
    std::fs::write(&binary, "#!/bin/sh\nprintf '%s\\n' 'herdr 0.9.4'\n").unwrap();
    assert_eq!(
        check_version(&binary).await.unwrap_err().code,
        "INCOMPATIBLE_HERDR"
    );
}
