//! GitHub Release updates. The readiness manifest is published last by CI.
use crate::{Error, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::Path,
    time::Duration,
};

const LIMIT: usize = 128 * 1024 * 1024;
const MANIFEST: &str = "herdr-relay-update.json";
/// Releases of the public source repository carry the update manifest.
const REPOSITORY: &str = "hackerchai/herdr-relay";
/// The release download site (Cloudflare R2), tried before GitHub Releases.
const SITE: &str = "https://herdr-relay-dl.hackerchai.com";

/// The repository to update from and whether it may need GitHub credentials.
fn source(repository: Option<&str>) -> (&str, bool) {
    let repository = repository.unwrap_or(REPOSITORY);
    (repository, repository != REPOSITORY)
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u8,
    version: String,
    artifacts: Vec<Artifact>,
}
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Artifact {
    target: String,
    asset: String,
    sha256: String,
}

fn target() -> Result<&'static str> {
    // Match the running executable's architecture, including Rosetta, rather
    // than silently switching the installation to another architecture.
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-musl"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-musl"),
        _ => Err(Error::new(
            "UPDATE_PLATFORM",
            "no release binary is available for this OS/architecture",
        )),
    }
}
fn select(releases: &[Release]) -> Result<&Release> {
    releases.iter().filter(|r| !r.draft && !r.prerelease && r.assets.iter().any(|a| a.name == MANIFEST))
        .filter_map(|r| semver::Version::parse(r.tag_name.strip_prefix('v')?).ok().filter(|v| v.pre.is_empty()).map(|v| (v,r)))
        .max_by(|a,b| a.0.cmp(&b.0)).map(|(_,r)| r)
        .ok_or_else(|| Error::new("UPDATE_UNAVAILABLE", "no complete stable release has an update manifest yet; retry after release publication finishes"))
}
struct GitHub {
    client: reqwest::Client,
    token: Option<String>,
}
impl GitHub {
    async fn new(authenticated: bool) -> Result<Self> {
        // The default public channel never needs a GitHub login. In particular,
        // an unrelated expired credential must not break ordinary updates.
        let mut token = authenticated
            .then(|| {
                std::env::var("GH_TOKEN")
                    .ok()
                    .or_else(|| std::env::var("GITHUB_TOKEN").ok())
                    .filter(|s| !s.is_empty())
            })
            .flatten();
        if authenticated
            && token.is_none()
            && let Ok(Ok(output)) = tokio::time::timeout(
                Duration::from_secs(5),
                tokio::process::Command::new("gh")
                    .args(["auth", "token", "--hostname", "github.com"])
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .output(),
            )
            .await
            && output.status.success()
        {
            token = String::from_utf8(output.stdout)
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
        }
        let client = reqwest::Client::builder()
            .user_agent("herdr-relay-updater")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(network)?;
        Ok(Self { client, token })
    }
    async fn get(&self, url: &str, binary: bool, limit: usize) -> Result<Vec<u8>> {
        let parsed = url::Url::parse(url).map_err(|_| invalid())?;
        if parsed.scheme() != "https"
            || parsed.host_str() != Some("api.github.com")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(invalid());
        }
        let mut request = self.client.get(url).header(
            "Accept",
            if binary {
                "application/octet-stream"
            } else {
                "application/vnd.github+json"
            },
        );
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let mut response = request.send().await.map_err(network)?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .and_then(|h| h.to_str().ok())
                .ok_or_else(invalid)?;
            let location = url::Url::parse(location).map_err(|_| invalid())?;
            if location.scheme() != "https"
                || !location
                    .host_str()
                    .is_some_and(|h| h.ends_with(".githubusercontent.com"))
            {
                return Err(invalid());
            }
            // GitHub's signed asset URL receives no repository credential.
            response = self.client.get(location).send().await.map_err(network)?;
        }
        if !response.status().is_success() {
            let code = response.status().as_u16();
            return Err(Error::new("UPDATE_DOWNLOAD", match code {
                401 | 404 => "GitHub Release access is unavailable. For a private repository, use gh auth login or your own GH_TOKEN with Contents: read. Public crate fallback: cargo install herdr-relay --locked --force.".to_owned(),
                403 | 429 => "GitHub denied access or rate-limited this request; check your repository permissions and retry later.".to_owned(),
                _ => format!("GitHub download failed (HTTP {code}); the installed version is unchanged"),
            }));
        }
        if response
            .content_length()
            .is_some_and(|size| size > limit as u64)
        {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            if bytes.len() + chunk.len() > limit {
                return Err(invalid());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
}

/// Where to look for releases, in order.
#[derive(Debug, PartialEq, Eq)]
enum Source {
    /// A download site serving `latest/` and `v{version}/` paths.
    Site(String),
    /// GitHub Releases of an owner/repository.
    GitHub(String),
}

fn sources(site: Option<&str>, repository: Option<&str>) -> Result<Vec<Source>> {
    if repository.is_some() {
        let (repository, _) = source(repository);
        return Ok(vec![Source::GitHub(repository.to_owned())]);
    }
    let site = site.unwrap_or(SITE);
    Site::new(site)?;
    Ok(vec![
        Source::Site(site.to_owned()),
        Source::GitHub(REPOSITORY.to_owned()),
    ])
}

/// The release download site: an HTTPS origin (loopback HTTP for tests).
struct Site {
    base: String,
    client: reqwest::Client,
}
impl Site {
    fn new(base: &str) -> Result<Self> {
        let url = url::Url::parse(base).map_err(|_| site_invalid())?;
        let loopback = url.scheme() == "http" && url.host_str() == Some("127.0.0.1");
        if !(url.scheme() == "https" || loopback)
            || url.origin().ascii_serialization() != base
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(site_invalid());
        }
        let client = reqwest::Client::builder()
            .user_agent("herdr-relay-updater")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(120))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(network)?;
        Ok(Self {
            base: base.to_owned(),
            client,
        })
    }
    async fn get(&self, path: &str, limit: usize) -> Result<Vec<u8>> {
        let mut response = self
            .client
            .get(format!("{}/{path}", self.base))
            .send()
            .await
            .map_err(network)?;
        if !response.status().is_success() {
            return Err(Error::new(
                "UPDATE_DOWNLOAD",
                format!("download site returned HTTP {}", response.status().as_u16()),
            ));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(network)? {
            if body.len() + chunk.len() > limit {
                return Err(invalid());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
    async fn manifest(&self) -> Result<Manifest> {
        serde_json::from_slice(&self.get(&format!("latest/{MANIFEST}"), 65536).await?)
            .map_err(|_| invalid())
    }
    async fn archive(&self, version: &str, asset: &str) -> Result<Vec<u8>> {
        self.get(&format!("v{version}/{asset}"), LIMIT).await
    }
}
fn site_invalid() -> Error {
    Error::new(
        "UPDATE_SOURCE",
        "the update download site must be an exact https:// origin",
    )
}

/// The last automatic check, kept in the profile.
pub const CHECK_CACHE: &str = "update-check.json";
#[derive(serde::Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checked {
    checked_at: u64,
    latest: String,
}

/// `HERDR_RELAY_UPDATE_CHECK` / `update-check`: `off`, or `<n>m`, `<n>h` or
/// `<n>d` of at least one hour. The default is one day.
pub fn check_interval(value: Option<&str>) -> Result<Option<Duration>> {
    let Some(value) = value else {
        return Ok(Some(Duration::from_secs(86400)));
    };
    if value == "off" {
        return Ok(None);
    }
    let invalid = || {
        Error::new(
            "UPDATE_CHECK",
            "use off or an interval such as 6h or 1d (at least 1h)",
        )
    };
    let (number, unit) = value.split_at(value.len().saturating_sub(1));
    let seconds = match unit {
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => return Err(invalid()),
    };
    let n: u64 = number
        .parse()
        .ok()
        .filter(|_| number.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(invalid)?;
    let total = n.checked_mul(seconds).ok_or_else(invalid)?;
    if total < 3600 {
        return Err(invalid());
    }
    Ok(Some(Duration::from_secs(total)))
}

fn read_checked(profile: &Path) -> Option<Checked> {
    let bytes = fs::read(profile.join(CHECK_CACHE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// A one-line prompt when the last check found a newer release.
pub fn notice(profile: &Path) -> Option<String> {
    let checked = read_checked(profile)?;
    let latest = semver::Version::parse(&checked.latest).ok()?;
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    (latest > current && latest.pre.is_empty())
        .then(|| format!("Update available: {current} → {latest}. Run herdr-relay update."))
}

/// The automatic check settings: environment, then config.toml.
pub struct Schedule {
    pub interval: Option<Duration>,
    pub site: Option<String>,
    pub repository: Option<String>,
}
pub fn schedule() -> Result<Schedule> {
    let config = crate::config::current();
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    Ok(Schedule {
        interval: check_interval(
            var("HERDR_RELAY_UPDATE_CHECK")
                .or(config.update_check)
                .as_deref(),
        )?,
        site: var("HERDR_RELAY_UPDATE_URL").or(config.update_url),
        repository: var("HERDR_RELAY_UPDATE_REPOSITORY").or(config.update_repository),
    })
}

/// The update prompt to show, unless automatic checks are turned off.
pub fn prompt(profile: &Path) -> Option<String> {
    schedule().ok()?.interval?;
    notice(profile)
}

/// Refresh the cached check in the background if it is due; never blocks or fails.
pub fn spawn_check(profile: &Path) {
    let Ok(Schedule {
        interval: Some(interval),
        site,
        repository,
    }) = schedule()
    else {
        return;
    };
    let profile = profile.to_owned();
    tokio::spawn(async move {
        let _ = tokio::time::timeout(
            Duration::from_secs(15),
            refresh_if_due(&profile, interval, site.as_deref(), repository.as_deref()),
        )
        .await;
    });
}

/// Check for a newer release if `interval` has passed since the last check.
/// Returns whether a check ran; a failed check keeps the previous answer.
pub async fn refresh_if_due(
    profile: &Path,
    interval: Duration,
    site: Option<&str>,
    repository: Option<&str>,
) -> Result<bool> {
    let sources = sources(site, repository)?;
    refresh_at(profile, interval, &sources, std::time::SystemTime::now()).await
}

async fn refresh_at(
    profile: &Path,
    interval: Duration,
    sources: &[Source],
    now: std::time::SystemTime,
) -> Result<bool> {
    let now = now
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_secs();
    if read_checked(profile).is_some_and(|c| now.saturating_sub(c.checked_at) < interval.as_secs())
    {
        return Ok(false);
    }
    let mut failure = None;
    for source in sources {
        let latest = match source {
            Source::Site(base) => Site::new(base)?.manifest().await.map(|m| m.version),
            Source::GitHub(repository) => latest_github(repository).await,
        };
        match latest.and_then(|v| {
            semver::Version::parse(&v).map_err(|_| invalid())?;
            Ok(v)
        }) {
            Ok(latest) => {
                let bytes = serde_json::to_vec(&Checked {
                    checked_at: now,
                    latest,
                })
                .map_err(|_| invalid())?;
                write_atomic(&profile.join(CHECK_CACHE), &bytes, 0o600)?;
                return Ok(true);
            }
            Err(error) => failure = Some(error),
        }
    }
    Err(failure.unwrap_or_else(invalid))
}

async fn latest_github(repository: &str) -> Result<String> {
    let (_, authenticated) = source(Some(repository));
    let github = GitHub::new(authenticated).await?;
    let bytes = github
        .get(
            &format!("https://api.github.com/repos/{repository}/releases?per_page=100"),
            false,
            4 * 1024 * 1024,
        )
        .await?;
    let releases: Vec<Release> = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let release = select(&releases)?;
    Ok(release
        .tag_name
        .strip_prefix('v')
        .ok_or_else(invalid)?
        .to_owned())
}

/// What a source offered: nothing to do, or a verified binary to install.
enum Fetched {
    Done,
    Install { version: String, binary: Vec<u8> },
}

pub async fn run(
    profile: &Path,
    check_only: bool,
    restart_service: bool,
    repository: Option<&str>,
    site: Option<&str>,
) -> Result<()> {
    let mut failure = None;
    for source in sources(site, repository)? {
        let fetched = match &source {
            Source::Site(base) => {
                println!("Checking the download site ({base})…");
                fetch_site(profile, check_only, restart_service, base).await
            }
            Source::GitHub(repository) => {
                println!("Checking GitHub Releases ({repository})…");
                fetch_github(profile, check_only, restart_service, repository).await
            }
        };
        match fetched {
            Ok(Fetched::Done) => return Ok(()),
            Ok(Fetched::Install { version, binary }) => {
                return install(profile, &version, &binary, restart_service).await;
            }
            // Nothing was changed yet, so the next source can be tried.
            Err(error) if matches!(source, Source::Site(_)) => {
                println!(
                    "Download site unavailable ({}); trying GitHub Releases.",
                    error.code
                );
                failure = Some(error);
            }
            Err(error) => return Err(error),
        }
    }
    Err(failure.unwrap_or_else(invalid))
}

/// Report the versions; `None` means nothing needs installing.
async fn decide(
    profile: &Path,
    check_only: bool,
    restart_service: bool,
    version: &str,
) -> Result<Option<()>> {
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?;
    let latest = semver::Version::parse(version).map_err(|_| invalid())?;
    println!("Current: {current}; latest complete stable release: {latest}.");
    if check_only {
        return Ok(None);
    }
    if latest < current {
        println!("The installed version is newer; no downgrade performed.");
        return Ok(None);
    }
    if latest == current
        && !restart_service
        && !crate::user_service::needs_upgrade(profile, version).await?
    {
        println!(
            "Already up to date. Use --restart-service to also apply a pending server update."
        );
        return Ok(None);
    }
    Ok(Some(()))
}

/// This platform's artifact in a manifest for `version`.
fn artifact<'a>(manifest: &'a Manifest, version: &str) -> Result<&'a Artifact> {
    if manifest.schema != 1 || manifest.version != version || manifest.artifacts.len() != 4 {
        return Err(invalid());
    }
    let platform = target()?;
    let artifact = manifest
        .artifacts
        .iter()
        .find(|a| a.target == platform)
        .ok_or_else(|| Error::new("UPDATE_PLATFORM", "release has no binary for this platform"))?;
    if artifact.asset != format!("herdr-relay-{version}-{platform}.tar.gz")
        || artifact.sha256.len() != 64
    {
        return Err(invalid());
    }
    Ok(artifact)
}

async fn fetch_site(
    profile: &Path,
    check_only: bool,
    restart_service: bool,
    base: &str,
) -> Result<Fetched> {
    let site = Site::new(base)?;
    let manifest = site.manifest().await?;
    let version = manifest.version.clone();
    semver::Version::parse(&version)
        .ok()
        .filter(|v| v.pre.is_empty())
        .ok_or_else(invalid)?;
    if decide(profile, check_only, restart_service, &version)
        .await?
        .is_none()
    {
        return Ok(Fetched::Done);
    }
    let artifact = artifact(&manifest, &version)?;
    println!("Downloading {}…", artifact.asset);
    let archive = site.archive(&version, &artifact.asset).await?;
    let binary = verified_binary(&archive, artifact, &version)?;
    Ok(Fetched::Install { version, binary })
}

async fn fetch_github(
    profile: &Path,
    check_only: bool,
    restart_service: bool,
    repository: &str,
) -> Result<Fetched> {
    let parts: Vec<_> = repository.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || !p
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
    {
        return Err(Error::new(
            "UPDATE_REPOSITORY",
            "use a GitHub owner/repository name",
        ));
    }
    let (_, authenticated) = source(Some(repository));
    let github = GitHub::new(authenticated).await?;
    let bytes = github
        .get(
            &format!("https://api.github.com/repos/{repository}/releases?per_page=100"),
            false,
            4 * 1024 * 1024,
        )
        .await?;
    let releases: Vec<Release> = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let release = select(&releases)?;
    let version = release.tag_name.strip_prefix('v').ok_or_else(invalid)?;
    if decide(profile, check_only, restart_service, version)
        .await?
        .is_none()
    {
        return Ok(Fetched::Done);
    }
    let manifest_asset = release
        .assets
        .iter()
        .find(|a| a.name == MANIFEST)
        .ok_or_else(invalid)?;
    let manifest: Manifest =
        serde_json::from_slice(&github.get(&manifest_asset.url, true, 65536).await?)
            .map_err(|_| invalid())?;
    let artifact = artifact(&manifest, version)?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == artifact.asset)
        .ok_or_else(invalid)?;
    println!("Downloading {}…", artifact.asset);
    let archive = github.get(&asset.url, true, LIMIT).await?;
    let binary = verified_binary(&archive, artifact, version)?;
    Ok(Fetched::Install {
        version: version.to_owned(),
        binary,
    })
}

async fn install(
    profile: &Path,
    version: &str,
    binary: &[u8],
    restart_service: bool,
) -> Result<()> {
    let executable =
        fs::canonicalize(std::env::current_exe().map_err(storage)?).map_err(storage)?;
    if executable
        .components()
        .any(|c| c.as_os_str() == "Cellar" || c.as_os_str() == "nix" || c.as_os_str() == "store")
    {
        return Err(Error::new(
            "UPDATE_MANAGED",
            "this installation is externally managed; upgrade it with its package manager",
        ));
    }
    let parent = executable.parent().ok_or_else(invalid)?;
    let meta = fs::metadata(&executable).map_err(storage)?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
        return Err(Error::new(
            "UPDATE_PERMISSIONS",
            "the executable is not owned by this user; upgrade using the original installation method",
        ));
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(parent.join(".herdr-relay-update.lock"))
        .map_err(storage)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(Error::new("UPDATE_BUSY", "another update is in progress"));
    }
    let mut stage = tempfile::Builder::new()
        .prefix(".herdr-relay-new-")
        .tempfile_in(parent)
        .map_err(storage)?;
    stage
        .as_file()
        .set_permissions(fs::Permissions::from_mode(meta.mode() & 0o777))
        .map_err(storage)?;
    stage
        .write_all(binary)
        .and_then(|_| stage.as_file().sync_all())
        .map_err(storage)?;
    let stage = stage.into_temp_path();
    let output = tokio::time::timeout(
        Duration::from_secs(10),
        tokio::process::Command::new(&stage)
            .arg("--version")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| invalid())?
    .map_err(storage)?;
    if !output.status.success()
        || String::from_utf8_lossy(&output.stdout).trim() != format!("herdr-relay {version}")
    {
        return Err(Error::new(
            "UPDATE_BINARY",
            "downloaded executable failed its version check; previous version preserved",
        ));
    }
    let old = fs::read(&executable).map_err(storage)?;
    let backup = parent.join(".herdr-relay-previous");
    write_atomic(&backup, &old, meta.mode() & 0o777)?;
    stage.persist(&executable).map_err(|e| storage(e.error))?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(storage)?;
    match crate::user_service::upgrade(profile, binary, restart_service).await {
        Ok(message) => println!("Updated to {version}. {message}"),
        Err(error) => {
            write_atomic(&executable, &old, meta.mode() & 0o777)?;
            return Err(error);
        }
    }
    println!("Previous executable: {}", backup.display());
    Ok(())
}

fn verified_binary(archive: &[u8], artifact: &Artifact, version: &str) -> Result<Vec<u8>> {
    if hex::encode(Sha256::digest(archive)) != artifact.sha256 {
        return Err(Error::new(
            "UPDATE_CHECKSUM",
            "release archive checksum does not match; installed version preserved",
        ));
    }
    let prefix = format!("herdr-relay-{version}-{}", artifact.target);
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    let mut binary = None;
    let mut platform = None;
    for entry in archive.entries().map_err(storage)? {
        let entry = entry.map_err(storage)?;
        let path = entry.path().map_err(storage)?.into_owned();
        if path != format!("{prefix}/bin/herdr-relay") && path != format!("{prefix}/TARGET") {
            continue;
        }
        if !entry.header().entry_type().is_file() {
            return Err(invalid());
        }
        let mut bytes = Vec::new();
        entry
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        if bytes.len() > LIMIT {
            return Err(invalid());
        }
        if path.ends_with("TARGET") {
            if platform.replace(bytes).is_some() {
                return Err(invalid());
            }
        } else if binary.replace(bytes).is_some() {
            return Err(invalid());
        }
    }
    if platform
        .as_deref()
        .and_then(|b| std::str::from_utf8(b).ok())
        .map(str::trim)
        != Some(artifact.target.as_str())
    {
        return Err(invalid());
    }
    binary.filter(|b| !b.is_empty()).ok_or_else(invalid)
}
fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().ok_or_else(invalid)?;
    let mut stage = tempfile::NamedTempFile::new_in(parent).map_err(storage)?;
    stage
        .as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .map_err(storage)?;
    stage
        .write_all(bytes)
        .and_then(|_| stage.as_file().sync_all())
        .map_err(storage)?;
    stage.persist(path).map_err(|e| storage(e.error))?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(storage)
}
fn invalid() -> Error {
    Error::new(
        "UPDATE_INVALID",
        "release metadata or archive is incomplete or invalid; installed version preserved",
    )
}
fn network(error: reqwest::Error) -> Error {
    Error::new(
        "UPDATE_NETWORK",
        if error.is_timeout() {
            "GitHub request timed out; retry later"
        } else {
            "GitHub request failed; check connectivity and retry"
        },
    )
}
fn storage(error: std::io::Error) -> Error {
    Error::new("UPDATE_IO", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_check_interval_defaults_to_a_day_and_can_be_turned_off() {
        let hour = Duration::from_secs(3600);
        assert_eq!(check_interval(None).unwrap(), Some(24 * hour));
        assert_eq!(check_interval(Some("off")).unwrap(), None);
        assert_eq!(check_interval(Some("6h")).unwrap(), Some(6 * hour));
        assert_eq!(check_interval(Some("2d")).unwrap(), Some(48 * hour));
        assert_eq!(
            check_interval(Some("90m")).unwrap(),
            Some(Duration::from_secs(5400))
        );
        for bad in ["30m", "0h", "h", "1w", "-1h", "", "24"] {
            assert_eq!(
                check_interval(Some(bad)).unwrap_err().code,
                "UPDATE_CHECK",
                "{bad:?}"
            );
        }
    }

    fn profile() -> tempfile::TempDir {
        let dir = tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        crate::identity::Identity::initialize(dir.path()).unwrap();
        dir
    }

    #[test]
    fn a_newer_cached_release_produces_a_notice() {
        let dir = profile();
        assert_eq!(notice(dir.path()), None);
        let cache = |latest: &str| {
            fs::write(
                dir.path().join(CHECK_CACHE),
                format!(r#"{{"checked_at":1,"latest":"{latest}"}}"#),
            )
            .unwrap();
        };
        cache("99.0.0");
        let text = notice(dir.path()).unwrap();
        assert!(
            text.contains("99.0.0") && text.contains("herdr-relay update"),
            "{text}"
        );
        cache(env!("CARGO_PKG_VERSION"));
        assert_eq!(notice(dir.path()), None);
        cache("0.0.1");
        assert_eq!(notice(dir.path()), None);
        cache("garbage");
        assert_eq!(notice(dir.path()), None);
    }

    #[tokio::test]
    async fn checks_run_only_when_the_interval_has_passed() {
        let dir = profile();
        let manifest = serde_json::json!({"schema":1,"version":"99.1.0","artifacts":[]});
        let base = site(vec![(
            "/latest/herdr-relay-update.json",
            manifest.to_string().into_bytes(),
        )])
        .await;
        let day = Duration::from_secs(86400);
        let start = std::time::UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        assert!(
            refresh_at(dir.path(), day, &[Source::Site(base.clone())], start)
                .await
                .unwrap()
        );
        assert!(notice(dir.path()).unwrap().contains("99.1.0"));
        // Within the interval nothing is fetched, even from an unreachable site.
        let later = start + day - Duration::from_secs(1);
        assert!(
            !refresh_at(
                dir.path(),
                day,
                &[Source::Site("http://127.0.0.1:9".into())],
                later
            )
            .await
            .unwrap()
        );
        // Once due, a failed check leaves the previous answer in place.
        let due = start + day;
        assert!(
            refresh_at(
                dir.path(),
                day,
                &[Source::Site("http://127.0.0.1:9".into())],
                due
            )
            .await
            .is_err()
        );
        assert!(notice(dir.path()).unwrap().contains("99.1.0"));
        assert!(
            refresh_at(dir.path(), day, &[Source::Site(base.clone())], due)
                .await
                .unwrap()
        );
    }

    #[test]
    fn the_download_site_comes_first_and_github_releases_are_the_fallback() {
        assert_eq!(
            sources(None, None).unwrap(),
            vec![
                Source::Site(SITE.into()),
                Source::GitHub("hackerchai/herdr-relay".into())
            ]
        );
        assert_eq!(
            sources(Some("https://mirror.example.org"), None).unwrap(),
            vec![
                Source::Site("https://mirror.example.org".into()),
                Source::GitHub("hackerchai/herdr-relay".into())
            ]
        );
        // A custom repository is used alone.
        assert_eq!(
            sources(None, Some("me/fork")).unwrap(),
            vec![Source::GitHub("me/fork".into())]
        );
        for bad in [
            "http://mirror.example.org",
            "https://x.example/path",
            "ftp://x",
        ] {
            assert_eq!(
                sources(Some(bad), None).unwrap_err().code,
                "UPDATE_SOURCE",
                "{bad}"
            );
        }
        assert!(sources(Some("http://127.0.0.1:9"), None).is_ok());
    }

    /// Serve fixed paths once each over loopback HTTP; anything else is 404.
    async fn site(files: Vec<(&'static str, Vec<u8>)>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let mut head = Vec::new();
                let mut byte = [0];
                while !head.ends_with(b"\r\n\r\n") {
                    if socket.read_exact(&mut byte).await.is_err() {
                        break;
                    }
                    head.push(byte[0]);
                }
                let line = String::from_utf8_lossy(&head).to_string();
                let path = line.split(' ').nth(1).unwrap_or("").to_owned();
                let (status, body) = match files.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", Vec::new()),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            }
        });
        base
    }

    #[tokio::test]
    async fn the_download_site_serves_the_latest_manifest_and_versioned_archives() {
        let manifest = serde_json::json!({"schema":1,"version":"9.9.9","artifacts":[]});
        let base = site(vec![
            (
                "/latest/herdr-relay-update.json",
                manifest.to_string().into_bytes(),
            ),
            ("/v9.9.9/herdr-relay-9.9.9-x.tar.gz", b"archive".to_vec()),
        ])
        .await;
        let site = Site::new(&base).unwrap();
        assert_eq!(site.manifest().await.unwrap().version, "9.9.9");
        assert_eq!(
            site.archive("9.9.9", "herdr-relay-9.9.9-x.tar.gz")
                .await
                .unwrap(),
            b"archive"
        );
        assert_eq!(
            site.archive("9.9.9", "missing.tar.gz")
                .await
                .unwrap_err()
                .code,
            "UPDATE_DOWNLOAD"
        );
        // An unreachable site is an ordinary download failure, so GitHub is tried next.
        let down = Site::new("http://127.0.0.1:9").unwrap();
        assert_eq!(down.manifest().await.unwrap_err().code, "UPDATE_NETWORK");
    }
    #[test]
    fn updates_come_from_the_public_source_repository_without_a_login() {
        assert_eq!(source(None), ("hackerchai/herdr-relay", false));
        // Any other repository, including the retired binary channel, is a
        // custom source that may need credentials.
        assert_eq!(
            source(Some("hackerchai/herdr-relay-releases")),
            ("hackerchai/herdr-relay-releases", true)
        );
        assert_eq!(source(Some("me/fork")), ("me/fork", true));
    }
    #[test]
    fn newest_complete_stable_release_wins_over_drafts_and_partial_uploads() {
        let releases: Vec<Release> = serde_json::from_value(serde_json::json!([
            {"tag_name":"v9.0.0","draft":false,"prerelease":false,"assets":[]},
            {"tag_name":"v1.2.0","draft":false,"prerelease":false,"assets":[{"name":MANIFEST,"url":"https://api.github.com/asset"}]},
            {"tag_name":"v2.0.0","draft":true,"prerelease":false,"assets":[{"name":MANIFEST,"url":"https://api.github.com/asset"}]}
        ])).unwrap();
        assert_eq!(select(&releases).unwrap().tag_name, "v1.2.0");
    }
    #[test]
    fn corrupt_download_cannot_be_extracted() {
        let artifact = Artifact {
            target: "aarch64-apple-darwin".into(),
            asset: "archive.tar.gz".into(),
            sha256: "0".repeat(64),
        };
        assert_eq!(
            verified_binary(b"truncated archive", &artifact, "1.0.0")
                .unwrap_err()
                .code,
            "UPDATE_CHECKSUM"
        );
    }
    #[test]
    fn valid_archives_require_one_binary_and_matching_platform() {
        fn archive(target: &str, duplicate: bool) -> (Vec<u8>, Artifact) {
            let compressor = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            let mut tar = tar::Builder::new(compressor);
            let prefix = "herdr-relay-1.0.0-aarch64-apple-darwin";
            let entries = if duplicate {
                vec![
                    ("TARGET", target.as_bytes()),
                    ("bin/herdr-relay", b"binary".as_slice()),
                    ("bin/herdr-relay", b"second".as_slice()),
                ]
            } else {
                vec![
                    ("TARGET", target.as_bytes()),
                    ("bin/herdr-relay", b"binary".as_slice()),
                ]
            };
            for (path, data) in entries {
                let mut header = tar::Header::new_gnu();
                header.set_mode(0o755);
                header.set_size(data.len() as u64);
                header.set_cksum();
                tar.append_data(&mut header, format!("{prefix}/{path}"), data)
                    .unwrap();
            }
            let bytes = tar.into_inner().unwrap().finish().unwrap();
            let artifact = Artifact {
                target: "aarch64-apple-darwin".into(),
                asset: format!("{prefix}.tar.gz"),
                sha256: hex::encode(Sha256::digest(&bytes)),
            };
            (bytes, artifact)
        }
        let (bytes, artifact) = archive("aarch64-apple-darwin", false);
        assert_eq!(
            verified_binary(&bytes, &artifact, "1.0.0").unwrap(),
            b"binary"
        );
        for (platform, duplicate) in [
            ("x86_64-apple-darwin", false),
            ("aarch64-apple-darwin", true),
        ] {
            let (bytes, artifact) = archive(platform, duplicate);
            assert_eq!(
                verified_binary(&bytes, &artifact, "1.0.0")
                    .unwrap_err()
                    .code,
                "UPDATE_INVALID"
            );
        }
    }
}
