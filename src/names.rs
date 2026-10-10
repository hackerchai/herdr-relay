//! Friendly labels are presentation metadata, never authentication identifiers.
use crate::{Error, Result, pairing::Invitation};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::Path,
};

pub fn valid_name(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 128
        && !value.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
}
pub fn hostname() -> String {
    hostname::get()
        .ok()
        .and_then(|n| n.into_string().ok())
        .filter(|n| valid_name(n))
        .unwrap_or_else(|| "My computer".into())
}
pub fn display(value: &str) -> String {
    value
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(128)
        .collect()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Token {
    version: u8,
    hostname: String,
    invitation: Invitation,
}
pub struct PairLink {
    pub invitation: Invitation,
    pub hostname: Option<String>,
}
impl PairLink {
    pub fn encode(invitation: Invitation, hostname: String) -> Result<String> {
        if !valid_name(&hostname) {
            return Err(invalid());
        }
        let origin = invitation.origin.clone();
        let token = Token {
            version: 1,
            hostname,
            invitation,
        };
        let bytes = serde_json::to_vec(&token).map_err(|_| invalid())?;
        Ok(format!(
            "{origin}/pair#token={}",
            URL_SAFE_NO_PAD.encode(bytes)
        ))
    }
    pub fn decode(value: &str) -> Result<Self> {
        let value = value.trim();
        if value.starts_with("hrp1:") {
            return Ok(Self {
                invitation: Invitation::decode(value)?,
                hostname: None,
            });
        }
        if value.len() > 8192 {
            return Err(invalid());
        }
        let url = url::Url::parse(value).map_err(|_| invalid())?;
        let fragment = url
            .fragment()
            .and_then(|s| s.strip_prefix("token="))
            .ok_or_else(invalid)?;
        if url.scheme() != "https"
            || url.path() != "/pair"
            || url.query().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid());
        }
        let bytes = URL_SAFE_NO_PAD.decode(fragment).map_err(|_| invalid())?;
        let token: Token = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if token.version != 1
            || !valid_name(&token.hostname)
            || url.origin().ascii_serialization() != token.invitation.origin
        {
            return Err(invalid());
        }
        let invitation = Invitation::decode(&token.invitation.encode()?)?;
        Ok(Self {
            invitation,
            hostname: Some(token.hostname),
        })
    }
}
fn invalid() -> Error {
    Error::new(
        "PAIR_INVALID",
        "paste a complete valid pairing link or legacy invitation",
    )
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostName {
    pub origin: String,
    pub identity: String,
    pub name: String,
    pub alias: String,
}

fn read(profile: &Path) -> Result<Vec<HostName>> {
    let path = profile.join("host-names.json");
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(storage(e)),
    };
    let m = file.metadata().map_err(storage)?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.len() > 65536
    {
        return Err(Error::new("HOST_NAMES", "invalid host-name storage"));
    }
    let mut bytes = Vec::new();
    file.take(65537).read_to_end(&mut bytes).map_err(storage)?;
    let records: Vec<HostName> = serde_json::from_slice(&bytes)
        .map_err(|_| Error::new("HOST_NAMES", "invalid host-name storage"))?;
    if records.len() > 256
        || records
            .iter()
            .any(|r| !valid_name(&r.name) || !valid_name(&r.alias))
    {
        return Err(Error::new("HOST_NAMES", "invalid host-name storage"));
    }
    Ok(records)
}

/// Calling with no name also migrates an old grant to a stable friendly label.
pub fn remember(
    profile: &Path,
    origin: &str,
    identity: &str,
    name: Option<&str>,
    alias: Option<&str>,
) -> Result<HostName> {
    crate::identity::private_directory(profile, false)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(profile.join("host-names.lock"))
        .map_err(storage)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(storage(std::io::Error::last_os_error()));
    }
    let mut records = read(profile)?;
    if name.is_some_and(|n| !valid_name(n)) || alias.is_some_and(|n| !valid_name(n)) {
        return Err(Error::new(
            "HOST_NAME",
            "use a name of 1-128 bytes without terminal control characters",
        ));
    }
    let index = records
        .iter()
        .position(|r| r.identity == identity && r.origin == origin);
    let mut record = index
        .map(|i| records[i].clone())
        .unwrap_or_else(|| HostName {
            origin: origin.into(),
            identity: identity.into(),
            name: name.unwrap_or("Paired host").into(),
            alias: String::new(),
        });
    if let Some(name) = name {
        record.name = name.into();
    }
    if let Some(alias) = alias {
        record.alias = alias.trim().into();
    }
    if record.alias.is_empty() {
        record.alias = record.name.clone();
        let mut suffix = 2;
        while records
            .iter()
            .any(|r| r.origin == origin && r.identity != identity && r.alias == record.alias)
        {
            record.alias = format!(
                "{} ({suffix})",
                record.name.chars().take(96).collect::<String>()
            );
            suffix += 1;
        }
    }
    if records
        .iter()
        .any(|r| r.origin == origin && r.identity != identity && r.alias == record.alias)
    {
        return Err(Error::new(
            "HOST_NAME",
            "another host already uses that alias",
        ));
    }
    if let Some(i) = index {
        records[i] = record.clone();
    } else {
        records.push(record.clone());
    }
    let bytes = serde_json::to_vec(&records)
        .map_err(|_| Error::new("HOST_NAMES", "cannot save host names"))?;
    if bytes.len() > 65536 || records.len() > 256 {
        return Err(Error::new("HOST_NAMES", "too many saved host labels"));
    }
    let mut stage = tempfile::NamedTempFile::new_in(profile).map_err(storage)?;
    stage
        .as_file()
        .set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(storage)?;
    stage
        .write_all(&bytes)
        .and_then(|_| stage.as_file().sync_all())
        .map_err(storage)?;
    stage
        .persist(profile.join("host-names.json"))
        .map_err(|e| storage(e.error))?;
    File::open(profile)
        .and_then(|f| f.sync_all())
        .map_err(storage)?;
    Ok(record)
}
fn storage(error: std::io::Error) -> Error {
    Error::new("HOST_NAMES", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_are_stable_distinct_and_cannot_contain_terminal_commands() {
        let temp = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        crate::identity::Identity::initialize(temp.path()).unwrap();
        let a = remember(
            temp.path(),
            "https://example.com",
            "a",
            Some("ubuntu"),
            None,
        )
        .unwrap();
        let b = remember(
            temp.path(),
            "https://example.com",
            "b",
            Some("ubuntu"),
            None,
        )
        .unwrap();
        assert_eq!(a.alias, "ubuntu");
        assert_eq!(b.alias, "ubuntu (2)");
        assert_eq!(
            remember(
                temp.path(),
                "https://example.com",
                "b",
                Some("renamed"),
                None
            )
            .unwrap()
            .alias,
            b.alias
        );
        assert!(
            remember(
                temp.path(),
                "https://example.com",
                "b",
                None,
                Some("ubuntu")
            )
            .is_err()
        );
        assert!(!valid_name("hello\x1b[2J"));
    }
    #[test]
    fn pairing_links_bind_origin_and_preserve_legacy_invitations() {
        let temp = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        crate::identity::Identity::initialize(temp.path()).unwrap();
        let invitation = crate::pairing::Store::open(temp.path(), "https://example.com")
            .unwrap()
            .invite(crate::pairing::now())
            .unwrap();
        let legacy = invitation.encode().unwrap();
        let link = PairLink::encode(invitation, "Ubuntu".into()).unwrap();
        assert_eq!(
            PairLink::decode(&link).unwrap().hostname.as_deref(),
            Some("Ubuntu")
        );
        assert_eq!(
            PairLink::decode(&legacy)
                .unwrap()
                .invitation
                .encode()
                .unwrap(),
            legacy
        );
        assert!(PairLink::decode(&link.replacen("example.com", "other.example", 1)).is_err());
        assert!(PairLink::decode(&format!("{link}&garbage")).is_err());
    }
}
