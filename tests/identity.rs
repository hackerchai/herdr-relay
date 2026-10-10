use herdr_relay::identity::Identity;
use std::os::unix::fs::PermissionsExt;

fn profile() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

#[test]
fn losing_the_private_key_never_silently_replaces_an_existing_identity() {
    let profile = profile();
    let identity = Identity::initialize(profile.path()).unwrap();
    std::fs::remove_file(profile.path().join("identity/device.pk8")).unwrap();
    assert_eq!(
        Identity::initialize(profile.path()).unwrap_err().code,
        "IDENTITY_MISSING"
    );
    assert!(!profile.path().join("identity/device.pk8").exists());
    assert_eq!(
        std::fs::read(profile.path().join("identity/device.der")).unwrap(),
        identity.certificate.as_ref()
    );
}

#[test]
fn device_identity_survives_reload_and_is_private_and_separate_per_profile() {
    let a = profile();
    let b = profile();
    let identity = Identity::initialize(a.path()).unwrap();
    assert_eq!(identity.id.len(), 64);
    assert_eq!(Identity::load(a.path()).unwrap().id, identity.id);
    assert_eq!(Identity::initialize(a.path()).unwrap().id, identity.id);
    assert_ne!(Identity::initialize(b.path()).unwrap().id, identity.id);
    assert_eq!(
        std::fs::metadata(a.path().join("identity/device.pk8"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(a.path().join("identity"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn identity_directory_cannot_alias_another_profile() {
    let a = profile();
    let b = profile();
    Identity::initialize(a.path()).unwrap();
    std::os::unix::fs::symlink(a.path().join("identity"), b.path().join("identity")).unwrap();
    assert_eq!(
        Identity::initialize(b.path()).unwrap_err().code,
        "UNSAFE_IDENTITY"
    );
}
