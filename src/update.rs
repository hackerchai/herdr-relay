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
const REPOSITORY: &str = "hackerchai/herdr-relay";

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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema: u8,
    version: String,
    artifacts: Vec<Artifact>,
}
#[derive(Deserialize)]
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
    async fn new() -> Result<Self> {
        let mut token = std::env::var("GH_TOKEN")
            .ok()
            .or_else(|| std::env::var("GITHUB_TOKEN").ok())
            .filter(|s| !s.is_empty());
        if token.is_none()
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

pub async fn run(
    profile: &Path,
    check_only: bool,
    restart_service: bool,
    repository: Option<&str>,
) -> Result<()> {
    let repository = repository.unwrap_or(REPOSITORY);
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
    let github = GitHub::new().await?;
    println!("Checking GitHub Releases ({repository})…");
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
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| invalid())?;
    let latest = semver::Version::parse(version).map_err(|_| invalid())?;
    println!("Current: {current}; latest complete stable release: {latest}.");
    if check_only {
        return Ok(());
    }
    if latest < current {
        println!("The installed version is newer; no downgrade performed.");
        return Ok(());
    }
    if latest == current
        && !restart_service
        && !crate::user_service::needs_upgrade(profile, version).await?
    {
        println!(
            "Already up to date. Use --restart-service to also apply a pending server update."
        );
        return Ok(());
    }
    let manifest_asset = release
        .assets
        .iter()
        .find(|a| a.name == MANIFEST)
        .ok_or_else(invalid)?;
    let manifest: Manifest =
        serde_json::from_slice(&github.get(&manifest_asset.url, true, 65536).await?)
            .map_err(|_| invalid())?;
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
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == artifact.asset)
        .ok_or_else(invalid)?;
    println!("Downloading {}…", artifact.asset);
    let archive = github.get(&asset.url, true, LIMIT).await?;
    let binary = verified_binary(&archive, artifact, version)?;
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
        .write_all(&binary)
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
    match crate::user_service::upgrade(profile, &binary, restart_service).await {
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
