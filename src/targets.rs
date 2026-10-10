//! Persistent, explicitly selected shared targets. Runtime sockets are never
//! persisted as proof that an instance is still online.
use crate::{
    Error, Result,
    control::random_id,
    identity::{Identity, private_directory},
    native::NativeTarget,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Locator {
    Session {
        config_dir: PathBuf,
        session: String,
    },
    Socket {
        path: PathBuf,
    },
}
impl Locator {
    pub fn resolve(&self) -> Result<NativeTarget> {
        match self {
            Self::Session {
                config_dir,
                session,
            } => NativeTarget::session(config_dir, session),
            Self::Socket { path } => NativeTarget::socket(path),
        }
    }
    fn socket_path(&self) -> PathBuf {
        match self {
            Self::Session {
                config_dir,
                session,
            } if session == "default" => config_dir.join("herdr-client.sock"),
            Self::Session {
                config_dir,
                session,
            } => config_dir
                .join("sessions")
                .join(session)
                .join("herdr-client.sock"),
            Self::Socket { path } => path.clone(),
        }
    }
    fn normalized(self) -> Result<Self> {
        // Reject unsafe paths before canonicalizing the macOS system aliases.
        self.resolve()?;
        match self {
            Self::Session {
                config_dir,
                session,
            } => Ok(Self::Session {
                config_dir: config_dir.canonicalize().map_err(storage)?,
                session,
            }),
            Self::Socket { path } => Ok(Self::Socket {
                path: path.canonicalize().map_err(storage)?,
            }),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub id: String,
    pub name: String,
    pub locator: Locator,
    pub enabled: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema: u8,
    device_id: String,
    targets: Vec<Definition>,
}
#[derive(Clone)]
pub struct Store {
    profile: PathBuf,
    device_id: String,
}
impl Store {
    pub fn open(profile: &Path) -> Result<Self> {
        let identity = Identity::load(profile)?;
        let store = Self {
            profile: profile.to_owned(),
            device_id: identity.id,
        };
        store.read()?;
        Ok(store)
    }
    pub fn list(&self) -> Result<Vec<Definition>> {
        Ok(self.read()?.targets)
    }
    pub fn add(&self, name: String, locator: Locator) -> Result<Definition> {
        valid_name(&name)?;
        let locator = locator.normalized()?;
        let native = locator.resolve()?;
        self.transaction(|state| {
            if state.targets.iter().any(|t| t.name == name) {
                return Err(Error::new(
                    "TARGET_NAME_CONFLICT",
                    "choose a unique target display name",
                ));
            }
            for target in &state.targets {
                if target.locator.socket_path() == locator.socket_path()
                    || target
                        .locator
                        .resolve()
                        .is_ok_and(|existing| existing.same_instance(&native))
                {
                    return Err(Error::new(
                        "TARGET_EXISTS",
                        "this Herdr instance is already configured, including disabled targets",
                    ));
                }
            }
            if state.targets.len() >= 8 {
                return Err(Error::new(
                    "QUOTA_EXCEEDED",
                    "at most eight targets may be configured",
                ));
            }
            let target = Definition {
                id: random_id()?,
                name,
                locator,
                enabled: true,
            };
            state.targets.push(target.clone());
            Ok(target)
        })
    }
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<()> {
        self.transaction(|state| {
            let target = state
                .targets
                .iter_mut()
                .find(|t| t.id == id)
                .ok_or_else(missing)?;
            target.enabled = enabled;
            Ok(())
        })
    }
    pub fn remove(&self, id: &str) -> Result<()> {
        self.transaction(|state| {
            let index = state
                .targets
                .iter()
                .position(|t| t.id == id)
                .ok_or_else(missing)?;
            state.targets.remove(index);
            Ok(())
        })
    }
    pub fn select(&self, selection: &str) -> Result<Definition> {
        let matches: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|t| t.id == selection || t.name == selection)
            .collect();
        match matches.len() {
            0 => Err(missing()),
            1 => Ok(matches.into_iter().next().unwrap()),
            _ => Err(Error::new(
                "AMBIGUOUS_TARGET",
                "selection matches multiple targets; choose another unique name or ID",
            )),
        }
    }
    fn read(&self) -> Result<State> {
        private_directory(&self.profile, false)?;
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.profile.join("targets.json"))
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(State {
                    schema: 1,
                    device_id: self.device_id.clone(),
                    targets: vec![],
                });
            }
            Err(error) => return Err(storage(error)),
        };
        private_file(&file)?;
        let mut bytes = Vec::new();
        file.take(65537).read_to_end(&mut bytes).map_err(storage)?;
        if bytes.len() > 65536 {
            return Err(corrupt());
        }
        let state: State = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
        if state.schema != 1 || state.device_id != self.device_id || state.targets.len() > 8 {
            return Err(corrupt());
        }
        for (i, target) in state.targets.iter().enumerate() {
            valid_name(&target.name)?;
            if target.id.len() != 32
                || !target
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || !target.locator.socket_path().is_absolute()
                || state.targets[..i]
                    .iter()
                    .any(|t| t.id == target.id || t.name == target.name)
            {
                return Err(corrupt());
            }
        }
        Ok(state)
    }
    fn transaction<T>(&self, change: impl FnOnce(&mut State) -> Result<T>) -> Result<T> {
        private_directory(&self.profile, false)?;
        let lock = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.profile.join("targets.lock"))
            .map_err(storage)?;
        private_file(&lock)?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
            return Err(storage(std::io::Error::last_os_error()));
        }
        let mut state = self.read()?;
        let result = change(&mut state)?;
        let bytes = serde_json::to_vec(&state).map_err(|_| corrupt())?;
        if bytes.len() > 65536 {
            return Err(Error::new(
                "QUOTA_EXCEEDED",
                "target configuration exceeds size limit",
            ));
        }
        let mut stage = tempfile::Builder::new()
            .prefix(".targets-")
            .permissions(std::fs::Permissions::from_mode(0o600))
            .tempfile_in(&self.profile)
            .map_err(storage)?;
        stage
            .write_all(&bytes)
            .and_then(|_| stage.as_file().sync_all())
            .map_err(storage)?;
        stage
            .persist(self.profile.join("targets.json"))
            .map_err(|e| storage(e.error))?;
        File::open(&self.profile)
            .and_then(|f| f.sync_all())
            .map_err(storage)?;
        Ok(result)
    }
}
fn valid_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return Err(Error::new(
            "INVALID_TARGET",
            "name must contain 1-128 bytes without control characters",
        ));
    }
    Ok(())
}
fn private_file(file: &File) -> Result<()> {
    let m = file.metadata().map_err(storage)?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.len() > 65536
    {
        return Err(corrupt());
    }
    Ok(())
}
fn missing() -> Error {
    Error::new(
        "TARGET_NOT_FOUND",
        "no configured target matches that selection",
    )
}
fn corrupt() -> Error {
    Error::new(
        "TARGET_STORAGE",
        "invalid or unsafe target state; it was not replaced",
    )
}
fn storage(error: std::io::Error) -> Error {
    Error::new("TARGET_STORAGE", error.to_string())
}

/// The service's current target set. Replacing a definition cancels only that
/// target's channels after the new configuration has committed to disk.
pub struct Registry {
    store: Store,
    updating: tokio::sync::Mutex<()>,
    live: std::sync::Mutex<Vec<std::sync::Arc<crate::host::SharedTarget>>>,
}
impl Registry {
    pub fn open(profile: &Path) -> Result<Self> {
        let store = Store::open(profile)?;
        let live = store
            .list()?
            .into_iter()
            .filter(|d| d.enabled)
            .map(|definition| {
                crate::host::SharedTarget::from_definition(definition).map(std::sync::Arc::new)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            store,
            updating: tokio::sync::Mutex::new(()),
            live: std::sync::Mutex::new(live),
        })
    }
    pub fn snapshot(&self) -> Vec<std::sync::Arc<crate::host::SharedTarget>> {
        self.live
            .lock()
            .expect("target registry lock poisoned")
            .clone()
    }
    pub async fn reload(&self) -> Result<bool> {
        let _update = self.updating.lock().await;
        let definitions = self.store.list()?;
        let live = self.snapshot();
        let mut next = Vec::new();
        for definition in definitions.into_iter().filter(|d| d.enabled) {
            if let Some(existing) = live
                .iter()
                .find(|t| t.definition.as_ref() == Some(&definition) && t.runtime_is_current())
            {
                next.push(existing.clone());
            } else {
                next.push(std::sync::Arc::new(
                    crate::host::SharedTarget::from_definition(definition)?,
                ));
            }
        }
        let retiring: Vec<_> = live
            .iter()
            .filter(|old| !next.iter().any(|new| std::sync::Arc::ptr_eq(old, new)))
            .collect();
        let changed = !retiring.is_empty() || next.len() != live.len();
        // Mark every old generation closed before awaiting any of them. The
        // next snapshot becomes visible only after all its native/TLS tasks
        // have actually dropped their resources, including pending handshakes.
        for old in &retiring {
            old.stop();
        }
        for old in retiring {
            old.drained().await;
        }
        *self.live.lock().map_err(|_| corrupt())? = next;
        Ok(changed)
    }
}
