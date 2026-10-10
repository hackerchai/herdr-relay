#![allow(dead_code)]
use herdr_relay::{
    control::{Context, Purpose},
    identity::Identity,
    pairing::{self, Store},
    targets::{Locator, Registry},
};
use std::{os::unix::fs::PermissionsExt, path::PathBuf, sync::Arc};
use tokio::net::UnixListener;

pub const ORIGIN: &str = "https://relay.example";

/// A host profile `a` sharing one native socket with a paired client `b`.
pub struct Fixture {
    pub temp: tempfile::TempDir,
    pub a: Identity,
    pub b: Identity,
    pub native: UnixListener,
    pub registry: Registry,
    pub target: Arc<herdr_relay::host::SharedTarget>,
}
impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let a = Identity::initialize(&temp.path().join("a")).unwrap();
        let b = Identity::initialize(&temp.path().join("b")).unwrap();
        let socket = temp.path().join("native.sock");
        let native = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        herdr_relay::targets::Store::open(&temp.path().join("a"))
            .unwrap()
            .add("work".into(), Locator::Socket { path: socket })
            .unwrap();
        let registry = Registry::open(&temp.path().join("a")).unwrap();
        let target = registry.snapshot()[0].clone();
        let store = Store::open(&temp.path().join("a"), ORIGIN).unwrap();
        let connector = Store::open(&temp.path().join("b"), ORIGIN).unwrap();
        let invitation = store.invite(pairing::now()).unwrap();
        let request = connector.prepare(&invitation, pairing::now()).unwrap();
        let receipt = store
            .redeem(&b.certificate, &request, pairing::now())
            .unwrap();
        connector
            .complete(&invitation, &request, &receipt, pairing::now())
            .unwrap();
        Self {
            temp,
            a,
            b,
            native,
            registry,
            target,
        }
    }
    pub fn host(&self) -> PathBuf {
        self.temp.path().join("a")
    }
    pub fn client(&self) -> PathBuf {
        self.temp.path().join("b")
    }
    pub fn context(&self, purpose: Purpose) -> Context {
        Context {
            version: 1,
            origin: ORIGIN.into(),
            publisher: self.a.id.clone(),
            connector: self.b.id.clone(),
            target: (purpose == Purpose::Attach).then(|| self.target.id.clone()),
            purpose,
            grant_version: Some(1),
            channel_id: "22".repeat(16),
            boot_id: "33".repeat(16),
        }
    }
}
