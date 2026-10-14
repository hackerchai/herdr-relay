use herdr_relay::client::check_version;
use std::{os::unix::fs::PermissionsExt, path::Path};

fn executable(path: &Path, script: &str) {
    std::fs::write(path, format!("#!/bin/sh\n{script}\n")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
}

#[tokio::test]
async fn native_releases_are_not_rejected_by_their_version_number() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("herdr");
    for banner in [
        "herdr 0.8.0",
        "herdr 0.9.3",
        "herdr 0.9.3-custom.relay-control-bound-1",
        "herdr 0.9.4",
        "herdr 0.9.5-preview.42",
        "herdr 0.10.0+local.abcdef",
        "herdr 1.0.0",
        "herdr 2.0.0 (custom build)",
        "herdr 0.9.4\r",
    ] {
        executable(
            &binary,
            &format!("[ \"$1\" = --version ] || exit 1\nprintf '%s\\n' '{banner}'"),
        );
        check_version(&binary)
            .await
            .unwrap_or_else(|error| panic!("{banner}: {error}"));
    }
}

#[tokio::test]
async fn invalid_or_failed_executables_still_fail_preflight() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("herdr");
    for script in [
        "printf 'other 0.9.4\\n'",
        "printf 'herdr\\n'",
        "printf 'herdr invalid\\n'",
        "printf 'herdr 0.9.4\\n'; exit 1",
        "printf 'herdr 0.9.4\\377'",
        "exit 0",
        // A valid prefix must not make oversized/truncated output acceptable.
        "printf 'herdr 0.9.4 '; i=0; while [ $i -lt 1100 ]; do printf x; i=$((i+1)); done",
    ] {
        executable(&binary, script);
        let error = check_version(&binary).await.unwrap_err();
        assert_eq!(error.code, "INCOMPATIBLE_HERDR", "{script}");
        assert!(!error.message.contains("use Herdr 0.9.3"));
    }
    assert_eq!(
        check_version(&directory.path().join("missing"))
            .await
            .unwrap_err()
            .code,
        "CLIENT_START_FAILED"
    );
}

#[tokio::test]
async fn a_stuck_version_command_is_bounded() {
    let directory = tempfile::tempdir().unwrap();
    let binary = directory.path().join("herdr");
    executable(&binary, "exec sleep 30");
    assert_eq!(
        check_version(&binary).await.unwrap_err().code,
        "CLIENT_START_TIMEOUT"
    );
}
