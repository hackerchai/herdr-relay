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
/// This computer's friendly name: the macOS computer name (as in Sharing
/// settings, stable across networks), otherwise the hostname without a
/// link-local suffix such as `.local`.
pub fn hostname() -> String {
    computer_name()
        .or_else(|| {
            hostname::get()
                .ok()
                .and_then(|n| n.into_string().ok())
                .and_then(|n| friendly(&n))
        })
        .unwrap_or_else(|| "My computer".into())
}
#[cfg(target_os = "macos")]
fn computer_name() -> Option<String> {
    let output = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    friendly(String::from_utf8(output.stdout).ok()?.trim())
}
#[cfg(not(target_os = "macos"))]
fn computer_name() -> Option<String> {
    None
}
/// A displayable name with `.local`, `.localdomain` or `.lan` removed.
pub fn friendly(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let lower = raw.to_ascii_lowercase();
    let name = [".local", ".localdomain", ".lan"]
        .iter()
        .find(|suffix| lower.ends_with(*suffix))
        .map_or(raw, |suffix| &raw[..raw.len() - suffix.len()]);
    valid_name(name).then(|| name.to_owned())
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostName {
    pub origin: String,
    pub identity: String,
    pub name: String,
    pub alias: String,
}

/// Hosts this device accesses (with local aliases), or devices that access it.
#[derive(Clone, Copy)]
struct Book {
    names: &'static str,
    lock: &'static str,
}
const HOSTS: Book = Book {
    names: "host-names.json",
    lock: "host-names.lock",
};
const PEERS: Book = Book {
    names: "peer-names.json",
    lock: "peer-names.lock",
};

fn read(profile: &Path, book: Book) -> Result<Vec<HostName>> {
    let path = profile.join(book.names);
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
    remember_in(profile, HOSTS, origin, identity, name, alias)
}

/// Apply the hostname a host reported inside its authenticated channel. An
/// alias the user never customized follows the new name.
pub fn refresh_host(profile: &Path, origin: &str, identity: &str, hostname: &str) -> Result<()> {
    if !valid_name(hostname) {
        return Err(Error::new("HOST_NAME", "invalid reported hostname"));
    }
    let current = read(profile, HOSTS)?
        .into_iter()
        .find(|r| r.origin == origin && r.identity == identity);
    // Only the name itself or its de-duplicated "name (N)" form is automatic.
    let automatic = current.as_ref().is_none_or(|r| {
        r.alias == r.name
            || r.alias
                .strip_prefix(&format!("{} (", r.name))
                .and_then(|rest| rest.strip_suffix(')'))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    });
    if current.as_ref().is_some_and(|r| r.name == hostname) {
        return Ok(());
    }
    remember_in(
        profile,
        HOSTS,
        origin,
        identity,
        Some(hostname),
        automatic.then_some(""),
    )
    .map(|_| ())
}

/// Record the hostname a paired client reported to this host.
pub fn remember_peer(profile: &Path, origin: &str, identity: &str, hostname: &str) -> Result<()> {
    if peer_name(profile, origin, identity).as_deref() == Some(hostname) {
        return Ok(());
    }
    remember_in(profile, PEERS, origin, identity, Some(hostname), Some("")).map(|_| ())
}
pub fn peer_name(profile: &Path, origin: &str, identity: &str) -> Option<String> {
    read(profile, PEERS)
        .ok()?
        .into_iter()
        .find(|r| r.origin == origin && r.identity == identity)
        .map(|r| r.name)
}

fn remember_in(
    profile: &Path,
    book: Book,
    origin: &str,
    identity: &str,
    name: Option<&str>,
    alias: Option<&str>,
) -> Result<HostName> {
    let _lock = lock(profile, book)?;
    let mut records = read(profile, book)?;
    // An empty alias resets to the automatic, name-derived alias.
    if name.is_some_and(|n| !valid_name(n))
        || alias.is_some_and(|n| !n.trim().is_empty() && !valid_name(n))
    {
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
            let mut base = record.name.clone();
            while base.len() > 96 {
                base.pop();
            }
            record.alias = format!("{base} ({suffix})");
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
    write(profile, book, &records)?;
    Ok(record)
}

/// Remove a host's saved name and alias.
pub fn forget(profile: &Path, origin: &str, identity: &str) -> Result<()> {
    let _lock = lock(profile, HOSTS)?;
    let mut records = read(profile, HOSTS)?;
    let before = records.len();
    records.retain(|r| !(r.origin == origin && r.identity == identity));
    if records.len() != before {
        write(profile, HOSTS, &records)?;
    }
    Ok(())
}

/// The saved host names, for listing.
pub fn hosts(profile: &Path) -> Result<Vec<HostName>> {
    read(profile, HOSTS)
}

fn lock(profile: &Path, book: Book) -> Result<File> {
    crate::identity::private_directory(profile, false)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(profile.join(book.lock))
        .map_err(storage)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) } != 0 {
        return Err(storage(std::io::Error::last_os_error()));
    }
    Ok(lock)
}

fn write(profile: &Path, book: Book, records: &[HostName]) -> Result<()> {
    let bytes = serde_json::to_vec(records)
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
        .persist(profile.join(book.names))
        .map_err(|e| storage(e.error))?;
    File::open(profile)
        .and_then(|f| f.sync_all())
        .map_err(storage)?;
    Ok(())
}
fn storage(error: std::io::Error) -> Error {
    Error::new("HOST_NAMES", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reported_names_drop_local_network_suffixes() {
        assert_eq!(
            friendly("Hackerchais-MacBook-Pro-2023.local").as_deref(),
            Some("Hackerchais-MacBook-Pro-2023")
        );
        assert_eq!(friendly("box.localdomain").as_deref(), Some("box"));
        assert_eq!(friendly("nas.LAN").as_deref(), Some("nas"));
        assert_eq!(
            friendly("herdr-relay-demo").as_deref(),
            Some("herdr-relay-demo")
        );
        // A real domain name is kept; only link-local suffixes are noise.
        assert_eq!(
            friendly("build.example.com").as_deref(),
            Some("build.example.com")
        );
        assert_eq!(
            friendly("Hackerchai's MacBook Pro 2023").as_deref(),
            Some("Hackerchai's MacBook Pro 2023")
        );
        for bad in ["", "  ", ".local", "evil\x1b[2J"] {
            assert_eq!(friendly(bad), None, "{bad:?}");
        }
    }
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
        let unicode = "主".repeat(42);
        remember(
            temp.path(),
            "https://example.com",
            "c",
            Some(&unicode),
            None,
        )
        .unwrap();
        let duplicate = remember(
            temp.path(),
            "https://example.com",
            "d",
            Some(&unicode),
            None,
        )
        .unwrap();
        assert!(valid_name(&duplicate.alias));
        assert_eq!(
            remember(temp.path(), "https://example.com", "d", None, None)
                .unwrap()
                .alias,
            duplicate.alias
        );
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
