use herdr_relay::{
    identity::Identity,
    targets::{Locator, Store},
};
use std::{
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    os::unix::net::UnixListener,
};

#[test]
fn shared_targets_keep_stable_identity_and_reject_duplicate_sessions_across_restart() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let profile = root.path().join("profile");
    Identity::initialize(&profile).unwrap();
    let config = root.path().join("herdr");
    let directory = config.join("sessions/work");
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&directory)
        .unwrap();
    let path = directory.join("herdr-client.sock");
    let _listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let store = Store::open(&profile).unwrap();
    let target = store
        .add(
            "Work".into(),
            Locator::Session {
                config_dir: config.clone(),
                session: "work".into(),
            },
        )
        .unwrap();
    assert!(target.enabled);
    assert_eq!(
        store
            .add("Alias".into(), Locator::Socket { path: path.clone() })
            .unwrap_err()
            .code,
        "TARGET_EXISTS"
    );
    store.set_enabled(&target.id, false).unwrap();
    let store = Store::open(&profile).unwrap();
    let list = store.list().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, target.id);
    assert_eq!(list[0].name, "Work");
    assert!(!list[0].enabled);
    std::fs::remove_file(&path).unwrap();
    store.set_enabled(&target.id, true).unwrap();
    assert!(
        store.list().unwrap()[0].enabled,
        "temporary offline state must not erase explicit sharing"
    );
    assert_eq!(
        store
            .add(
                "Absent".into(),
                Locator::Session {
                    config_dir: config,
                    session: "personal".into()
                }
            )
            .unwrap_err()
            .code,
        "TARGET_OFFLINE"
    );
    store.remove(&target.id).unwrap();
    assert!(store.list().unwrap().is_empty());
    assert_eq!(
        store.remove(&target.id).unwrap_err().code,
        "TARGET_NOT_FOUND"
    );
}

#[tokio::test]
async fn registry_tracks_only_saved_enabled_runtime_generations() {
    use herdr_relay::targets::Registry;
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let profile = root.path().join("profile");
    Identity::initialize(&profile).unwrap();
    let work = root.path().join("work.sock");
    let other = root.path().join("other.sock");
    let first = UnixListener::bind(&work).unwrap();
    let _other = UnixListener::bind(&other).unwrap();
    for path in [&work, &other] {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let store = Store::open(&profile).unwrap();
    let saved = store
        .add("Work".into(), Locator::Socket { path: work.clone() })
        .unwrap();
    store
        .add("Other".into(), Locator::Socket { path: other })
        .unwrap();
    let registry = Registry::open(&profile).unwrap();
    let old = registry.snapshot();
    let old_runtime = old[0].status().runtime_id;
    drop(first);
    std::fs::remove_file(&work).unwrap();
    registry.reload().await.unwrap();
    assert_eq!(registry.snapshot()[0].status().status, "offline");
    let second = UnixListener::bind(&work).unwrap();
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o600)).unwrap();
    registry.reload().await.unwrap();
    let live = registry.snapshot();
    assert_eq!(live[0].id, saved.id);
    assert_eq!(live[0].status().status, "online");
    assert_ne!(live[0].status().runtime_id, old_runtime);
    assert!(
        std::sync::Arc::ptr_eq(&old[1], &live[1]),
        "another target must keep its runtime"
    );
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o666)).unwrap();
    registry.reload().await.unwrap();
    assert_eq!(registry.snapshot()[0].status().status, "offline");
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o600)).unwrap();
    registry.reload().await.unwrap();
    assert_eq!(registry.snapshot()[0].status().status, "online");
    store.set_enabled(&saved.id, false).unwrap();
    registry.reload().await.unwrap();
    drop(second);
    std::fs::remove_file(&work).unwrap();
    let _third = UnixListener::bind(&work).unwrap();
    registry.reload().await.unwrap();
    assert!(
        registry.snapshot().iter().all(|t| t.id != saved.id),
        "restart cannot re-enable an explicitly disabled target"
    );
}

#[test]
fn interactive_selection_is_atomic_and_rejects_stale_edits() {
    let root = tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap();
    let profile = root.path().join("profile");
    Identity::initialize(&profile).unwrap();
    let store = Store::open(&profile).unwrap();
    let path = root.path().join("one.sock");
    let listener = UnixListener::bind(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let locator = Locator::Socket { path: path.clone() };
    store
        .apply_selection(&[], &[("One".into(), locator.clone())])
        .unwrap();
    let original = store.list().unwrap();
    let absent = Locator::Socket {
        path: root.path().join("missing.sock"),
    };
    assert!(
        store
            .apply_selection(&original, &[("Missing".into(), absent)])
            .is_err()
    );
    assert_eq!(
        store.list().unwrap(),
        original,
        "a failed batch cannot disable an existing share"
    );
    store.set_enabled(&original[0].id, false).unwrap();
    assert_eq!(
        store.apply_selection(&original, &[]).unwrap_err().code,
        "SHARES_CHANGED"
    );
    drop(listener);
    std::fs::remove_file(path).unwrap();
    let current = store.list().unwrap();
    store
        .apply_selection(&current, &[("One".into(), current[0].locator.clone())])
        .unwrap();
    let restored = store.list().unwrap();
    assert_eq!(restored[0].id, original[0].id);
    assert!(
        restored[0].enabled,
        "saved offline sessions remain selectable"
    );
}
