//! Persistent CLI settings in `config.toml`.
//!
//! Precedence is command-line flag, then environment variable, then this file,
//! then the built-in default. Relay access tokens are stored per origin and are
//! only ever sent to that origin; the public hosted relay never uses one.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::RwLock,
};

/// The public hosted relay. It is open to everyone and never receives a token.
pub const PUBLIC_ORIGIN: &str = "https://herdr-relay.hackerchai.com";
/// Keys accepted by `config set` and `config unset`.
pub const KEYS: &[&str] = &[
    "origin",
    "profile",
    "herdr",
    "update-repository",
    "direct",
    "direct-port",
    "token",
];

#[derive(Default, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub herdr: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_repository: Option<String>,
    /// Whether a host started with this configuration listens for direct connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct_port: Option<u16>,
    /// Relay access tokens keyed by exact relay origin.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tokens: BTreeMap<String, String>,
}

impl Config {
    /// Read a configuration file; a missing file is an empty configuration.
    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(Error::new(
                    "CONFIG_INVALID",
                    format!("{}: {e}", path.display()),
                ));
            }
        };
        let config: Self = toml::from_str(&text)
            .map_err(|e| Error::new("CONFIG_INVALID", format!("{}: {e}", path.display())))?;
        config.validate().map_err(|e| {
            Error::new(
                "CONFIG_INVALID",
                format!("{}: {}", path.display(), e.message),
            )
        })?;
        Ok(config)
    }

    /// Atomically write the configuration, readable only by its owner.
    pub fn save(&self, path: &Path) -> Result<()> {
        let io =
            |e: std::io::Error| Error::new("CONFIG_STORAGE", format!("{}: {e}", path.display()));
        let text =
            toml::to_string(self).map_err(|e| Error::new("CONFIG_STORAGE", e.to_string()))?;
        let dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir).map_err(io)?;
        let mut stage = tempfile::NamedTempFile::new_in(dir).map_err(io)?;
        stage
            .as_file()
            .set_permissions(std::fs::Permissions::from_mode(0o600))
            .map_err(io)?;
        stage
            .write_all(text.as_bytes())
            .and_then(|_| stage.as_file().sync_all())
            .map_err(io)?;
        stage.persist(path).map_err(|e| io(e.error))?;
        Ok(())
    }

    /// Change one setting. `origin` selects the relay a token belongs to.
    pub fn set(&mut self, key: &str, value: &str, origin: Option<&str>) -> Result<()> {
        let mut next = self.clone();
        match key {
            "origin" => next.origin = Some(value.to_owned()),
            "profile" => next.profile = Some(PathBuf::from(value)),
            "herdr" => next.herdr = Some(PathBuf::from(value)),
            "update-repository" => next.update_repository = Some(value.to_owned()),
            "direct" => {
                next.direct = Some(match value {
                    "on" | "true" | "yes" => true,
                    "off" | "false" | "no" => false,
                    _ => return Err(setting("direct is on or off")),
                })
            }
            "direct-port" => {
                next.direct_port = Some(
                    value
                        .parse()
                        .ok()
                        .filter(|port| *port != 0)
                        .ok_or_else(|| setting("direct-port is a TCP port from 1 to 65535"))?,
                )
            }
            "token" => {
                let origin = token_origin(origin)?;
                next.tokens.insert(origin.to_owned(), value.to_owned());
            }
            _ => return Err(unknown_key(key)),
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Remove one setting so its environment variable or default applies.
    pub fn unset(&mut self, key: &str, origin: Option<&str>) -> Result<()> {
        match key {
            "origin" => self.origin = None,
            "profile" => self.profile = None,
            "herdr" => self.herdr = None,
            "update-repository" => self.update_repository = None,
            "direct" => self.direct = None,
            "direct-port" => self.direct_port = None,
            "token" => {
                let origin = origin.ok_or_else(|| setting("choose the relay with --origin"))?;
                self.tokens.remove(origin);
            }
            _ => return Err(unknown_key(key)),
        }
        Ok(())
    }

    /// The token configured for exactly this origin.
    pub fn token(&self, origin: &str) -> Option<&str> {
        self.tokens.get(origin).map(String::as_str)
    }

    /// A human-readable listing that never reveals a token.
    pub fn show(&self) -> String {
        let mut lines = Vec::new();
        let mut line = |key: &str, value: Option<String>| {
            if let Some(value) = value {
                lines.push(format!("{key} = {value}"));
            }
        };
        line("origin", self.origin.clone());
        line(
            "profile",
            self.profile.as_ref().map(|p| p.display().to_string()),
        );
        line(
            "herdr",
            self.herdr.as_ref().map(|p| p.display().to_string()),
        );
        line("update-repository", self.update_repository.clone());
        line(
            "direct",
            self.direct.map(|on| if on { "on" } else { "off" }.into()),
        );
        line("direct-port", self.direct_port.map(|p| p.to_string()));
        for origin in self.tokens.keys() {
            lines.push(format!("token for {origin} = (set)"));
        }
        if lines.is_empty() {
            "(no settings)".into()
        } else {
            lines.join("\n")
        }
    }

    fn validate(&self) -> Result<()> {
        if let Some(origin) = &self.origin {
            crate::pairing::validate_origin(origin)
                .map_err(|_| setting("origin must be an exact https:// origin"))?;
        }
        if let Some(repository) = &self.update_repository {
            let parts: Vec<_> = repository.split('/').collect();
            if parts.len() != 2
                || parts.iter().any(|p| {
                    p.is_empty()
                        || !p
                            .bytes()
                            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
                })
            {
                return Err(setting(
                    "update-repository is a GitHub owner/repository name",
                ));
            }
        }
        if self.direct_port == Some(0) {
            return Err(setting("direct-port is a TCP port from 1 to 65535"));
        }
        for (origin, token) in &self.tokens {
            token_origin(Some(origin))?;
            validate_token(token)?;
        }
        Ok(())
    }
}

/// A relay access token: 16 to 512 visible ASCII characters.
pub fn validate_token(token: &str) -> Result<()> {
    if !(16..=512).contains(&token.len()) || !token.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(setting(
            "a relay token is 16 to 512 visible ASCII characters without spaces",
        ));
    }
    Ok(())
}

fn token_origin(origin: Option<&str>) -> Result<&str> {
    let origin = origin.ok_or_else(|| setting("choose the relay with --origin"))?;
    if origin == PUBLIC_ORIGIN {
        return Err(Error::new(
            "PUBLIC_RELAY",
            "the public relay is open to everyone and never uses a token",
        ));
    }
    crate::pairing::validate_origin(origin)
        .map_err(|_| setting("a token belongs to an exact https:// relay origin"))?;
    Ok(origin)
}

fn setting(message: &str) -> Error {
    Error::new("CONFIG_INVALID", message)
}

fn unknown_key(key: &str) -> Error {
    Error::new(
        "CONFIG_INVALID",
        format!("unknown setting {key:?}; use one of {}", KEYS.join(", ")),
    )
}

/// `HERDR_RELAY_CONFIG`, else `$XDG_CONFIG_HOME/herdr-relay/config.toml`,
/// else `~/.config/herdr-relay/config.toml`.
pub fn path() -> Result<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    if let Some(path) = var("HERDR_RELAY_CONFIG") {
        return Ok(PathBuf::from(path));
    }
    if let Some(root) = var("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(root).join("herdr-relay/config.toml"));
    }
    var("HOME")
        .map(|home| PathBuf::from(home).join(".config/herdr-relay/config.toml"))
        .ok_or_else(|| {
            Error::new(
                "CONFIG_INVALID",
                "set HERDR_RELAY_CONFIG when HOME is unavailable",
            )
        })
}

static CURRENT: RwLock<Option<Config>> = RwLock::new(None);

/// The configuration this process uses; set once at startup and after edits.
pub fn current() -> Config {
    CURRENT
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .unwrap_or_default()
}

pub fn use_config(config: Config) {
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(config);
}

/// Save a token entered interactively and use it in this process at once.
pub fn save_token(path: &Path, origin: &str, token: &str) -> Result<()> {
    let mut config = Config::load(path)?;
    config.set("token", token, Some(origin))?;
    config.save(path)?;
    let mut current = current();
    current.tokens.insert(origin.to_owned(), token.to_owned());
    use_config(current);
    Ok(())
}

/// The token to present to `origin`: `HERDR_RELAY_TOKEN`, else the configured
/// token for that origin. The public relay never receives one.
pub fn access_token(origin: &str) -> Option<String> {
    if origin == PUBLIC_ORIGIN {
        return None;
    }
    std::env::var("HERDR_RELAY_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .or_else(|| current().token(origin).map(str::to_owned))
}
