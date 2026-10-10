use herdr_relay::native::NativeTarget;
use std::{
    fs,
    os::unix::{
        fs::{DirBuilderExt, PermissionsExt, symlink},
        net::UnixListener,
    },
};

fn private_listener(path: impl AsRef<std::path::Path>) -> std::io::Result<UnixListener> {
    let socket = UnixListener::bind(path.as_ref())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(socket)
}

#[test]
fn explicitly_selected_session_never_falls_back_to_default() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let _default = private_listener(root.path().join("herdr-client.sock")).unwrap();
    let error = NativeTarget::session(root.path(), "work").unwrap_err();
    assert_eq!(error.code, "TARGET_OFFLINE");

    let session = root.path().join("sessions/work");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&session)
        .unwrap();
    let _work = private_listener(session.join("herdr-client.sock")).unwrap();
    let target = NativeTarget::session(root.path(), "work").unwrap();
    assert_eq!(target.path, session.join("herdr-client.sock"));
}

#[test]
fn session_spelling_is_exact_even_on_case_insensitive_filesystems() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let directory = root.path().join("sessions/Work");
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let _socket = private_listener(directory.join("herdr-client.sock")).unwrap();
    assert_eq!(
        NativeTarget::session(root.path(), "work").unwrap_err().code,
        "TARGET_OFFLINE"
    );
    assert!(NativeTarget::session(root.path(), "Work").is_ok());
}

#[test]
fn a_second_relay_attach_is_busy_until_the_first_reservation_is_dropped() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let _socket = private_listener(root.path().join("herdr-client.sock")).unwrap();
    let target = NativeTarget::session(root.path(), "default").unwrap();
    let first = target.reserve().unwrap();
    assert_eq!(target.reserve().unwrap_err().code, "TARGET_BUSY");
    drop(first);
    assert!(target.reserve().is_ok());
}

#[test]
fn target_must_be_a_private_owned_socket_without_user_symlinks() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let path = root.path().join("herdr-client.sock");
    fs::write(&path, "not a socket").unwrap();
    assert_eq!(
        NativeTarget::session(root.path(), "default")
            .unwrap_err()
            .code,
        "UNSAFE_TARGET"
    );
    fs::remove_file(&path).unwrap();
    let actual = root.path().join("actual.sock");
    let _socket = private_listener(&actual).unwrap();
    symlink(&actual, &path).unwrap();
    assert_eq!(
        NativeTarget::session(root.path(), "default")
            .unwrap_err()
            .code,
        "UNSAFE_TARGET"
    );
    fs::remove_file(&path).unwrap();
    let _socket = private_listener(&path).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap();
    assert_eq!(
        NativeTarget::session(root.path(), "default")
            .unwrap_err()
            .code,
        "UNSAFE_TARGET"
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = root.path().join("alias");
    symlink(root.path(), &alias).unwrap();
    assert_eq!(
        NativeTarget::session(&alias, "default").unwrap_err().code,
        "UNSAFE_TARGET"
    );
    assert!(NativeTarget::session(root.path(), "default").is_ok());
}

#[test]
fn session_selection_rejects_traversal_and_ambiguous_names() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    for name in [
        "",
        "..",
        ".",
        "../work",
        "work/personal",
        "work session",
        "工作",
    ] {
        assert_eq!(
            NativeTarget::session(root.path(), name).unwrap_err().code,
            "INVALID_TARGET",
            "{name}"
        );
    }
}
