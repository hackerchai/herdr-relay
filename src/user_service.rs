//! Own only a profile-specific user job and its installed executable.
use crate::{
    Error, Result,
    identity::{Identity, private_directory, user_directory},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Stdio,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    version: u8,
    origin: String,
    start_at_login: bool,
}
struct Paths {
    directory: PathBuf,
    binary: PathBuf,
    config: PathBuf,
    definition: PathBuf,
    label: String,
}
impl Paths {
    fn new(profile: &Path) -> Result<Self> {
        private_directory(profile, false)?;
        let canonical = fs::canonicalize(profile).map_err(io)?;
        if canonical.starts_with("/private/tmp")
            || canonical.starts_with("/tmp")
            || canonical.starts_with("/run")
            || canonical.starts_with("/dev/shm")
            || canonical.starts_with("/private/var/folders")
            || canonical.starts_with("/private/var/tmp")
            || canonical.starts_with("/var/tmp")
        {
            return Err(Error::new(
                "SERVICE_PATH",
                "background services require a persistent profile outside temporary directories",
            ));
        }
        let label = format!(
            "dev.herdr.relay.{}",
            &hex::encode(Sha256::digest(canonical.as_os_str().as_encoded_bytes()))[..16]
        );
        let directory = canonical.join("service");
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| Error::new("SERVICE_PATH", "HOME is unavailable"))?;
        let definition = if cfg!(target_os = "linux") {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("systemd/user")
                .join(format!("{label}.service"))
        } else {
            home.join("Library/LaunchAgents")
                .join(format!("{label}.plist"))
        };
        if exists(&directory)? {
            private_directory(&directory, false)?;
        }
        if exists(definition.parent().unwrap())? {
            user_directory(definition.parent().unwrap(), false)?;
        }
        Ok(Self {
            binary: directory.join("herdr-relay"),
            config: directory.join("config.json"),
            directory,
            definition,
            label,
        })
    }
    fn target(&self) -> String {
        if cfg!(target_os = "linux") {
            format!("{}.service", self.label)
        } else {
            format!("gui/{}/{}", unsafe { libc::geteuid() }, self.label)
        }
    }
    fn installed(&self) -> Result<bool> {
        if !exists(&self.config)? {
            return Ok(false);
        }
        private_directory(&self.directory, false)?;
        let mut data = Vec::new();
        safe_file(&self.config)?
            .take(8193)
            .read_to_end(&mut data)
            .map_err(io)?;
        let c: Config = serde_json::from_slice(&data)
            .map_err(|_| Error::new("SERVICE_CONFIG", "invalid installed service configuration"))?;
        if data.len() > 8192 || c.version != 1 {
            return Err(Error::new(
                "SERVICE_CONFIG",
                "unsupported installed service configuration",
            ));
        }
        safe_file(&self.definition)?;
        safe_file(&self.binary)?;
        Ok(true)
    }
}
fn supported() -> Result<()> {
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        Ok(())
    } else {
        Err(Error::new(
            "SERVICE_UNSUPPORTED",
            "use foreground serve on this platform",
        ))
    }
}
fn exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(io(e)),
    }
}
fn safe_file(path: &Path) -> Result<fs::File> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(io)?;
    let m = f.metadata().map_err(io)?;
    if !m.is_file() || m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o077 != 0 {
        return Err(Error::new(
            "UNSAFE_SERVICE",
            "service files must be private regular files owned by this user",
        ));
    }
    Ok(f)
}
fn atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    if exists(path)? {
        safe_file(path)?;
    }
    let parent = path
        .parent()
        .ok_or_else(|| Error::new("SERVICE_PATH", "missing service parent directory"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(io)?;
    temp.as_file()
        .set_permissions(fs::Permissions::from_mode(mode))
        .map_err(io)?;
    temp.write_all(bytes).map_err(io)?;
    temp.as_file().sync_all().map_err(io)?;
    temp.persist(path).map_err(|e| io(e.error))?;
    fs::File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(io)
}
fn xml(value: &str) -> Result<String> {
    if value.chars().any(|c| c < ' ' || c == '\u{7f}') {
        return Err(Error::new(
            "SERVICE_PATH",
            "service values must not contain control characters",
        ));
    }
    Ok(value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;"))
}
fn definition(paths: &Paths, profile: &Path, origin: &str, login: bool) -> Result<String> {
    if cfg!(target_os = "linux") {
        return linux_definition(paths, profile, origin);
    }
    let arguments = [
        paths.binary.to_str(),
        Some("--profile"),
        profile.to_str(),
        Some("serve"),
        Some("--relay"),
        Some("--origin"),
        Some(origin),
    ]
    .into_iter()
    .map(|s| {
        s.ok_or_else(|| Error::new("SERVICE_PATH", "service paths must be UTF-8"))
            .and_then(xml)
            .map(|v| format!("<string>{v}</string>"))
    })
    .collect::<Result<Vec<_>>>()?
    .join("");
    let mut environment = String::new();
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        if let Ok(value) = std::env::var(key) {
            environment.push_str(&format!(
                "<key>{key}</key><string>{}</string>",
                xml(&value)?
            ));
        }
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array>{arguments}</array><key>EnvironmentVariables</key><dict>{environment}</dict><key>RunAtLoad</key><{}/><key>Umask</key><integer>63</integer><key>ExitTimeOut</key><integer>15</integer><key>StandardOutPath</key><string>/dev/null</string><key>StandardErrorPath</key><string>/dev/null</string></dict></plist>\n",
        paths.label,
        if login { "true" } else { "false" }
    ))
}
fn systemd_quote(value: &str) -> Result<String> {
    if value.chars().any(|c| c < ' ' || c == '\u{7f}') {
        return Err(Error::new(
            "SERVICE_PATH",
            "service values must not contain control characters",
        ));
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    ))
}
fn linux_definition(paths: &Paths, profile: &Path, origin: &str) -> Result<String> {
    let arguments = [
        paths.binary.to_str(),
        Some("--profile"),
        profile.to_str(),
        Some("serve"),
        Some("--relay"),
        Some("--origin"),
        Some(origin),
    ]
    .into_iter()
    .map(|s| {
        s.ok_or_else(|| Error::new("SERVICE_PATH", "service paths must be UTF-8"))
            .and_then(systemd_quote)
    })
    .collect::<Result<Vec<_>>>()?
    .join(" ");
    let mut environment = String::new();
    for key in [
        "HOME",
        "XDG_CONFIG_HOME",
        "XDG_STATE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        if let Ok(value) = std::env::var(key) {
            environment.push_str(&format!(
                "Environment={}\n",
                systemd_quote(&format!("{key}={value}"))?
            ));
        }
    }
    // ':' disables ExecStart environment substitution. Percent specifiers are
    // escaped independently; no command is evaluated by a shell.
    Ok(format!(
        "[Unit]\nDescription=Herdr Relay user service\nStartLimitIntervalSec=60\nStartLimitBurst=3\n\n[Service]\nType=exec\nExecStart=:{arguments}\n{environment}UMask=0077\nRestart=on-failure\nRestartSec=5s\nTimeoutStopSec=15s\nStandardOutput=null\nStandardError=journal\n\n[Install]\nWantedBy=default.target\n"
    ))
}
async fn launch(args: &[&str]) -> Result<std::process::Output> {
    let mut command = tokio::process::Command::new(if cfg!(target_os = "linux") {
        "/usr/bin/systemctl"
    } else {
        "/bin/launchctl"
    });
    if cfg!(target_os = "linux") {
        command.arg("--user");
    }
    command.args(args).stdin(Stdio::null()).kill_on_drop(true);
    tokio::time::timeout(std::time::Duration::from_secs(20), command.output())
        .await
        .map_err(|_| Error::new("SERVICE_MANAGER", "user service manager timed out"))?
        .map_err(|_| {
            Error::new(
                "SERVICE_MANAGER",
                "user service manager unavailable; use foreground serve",
            )
        })
}
async fn loaded(paths: &Paths) -> Result<bool> {
    if cfg!(target_os = "linux") {
        let r = launch(&[
            "show",
            "--property=LoadState",
            "--property=ActiveState",
            &paths.target(),
        ])
        .await?;
        if !r.status.success() {
            return Err(Error::new(
                "SERVICE_MANAGER",
                "systemd user session unavailable; use foreground serve",
            ));
        }
        return Ok(String::from_utf8_lossy(&r.stdout).lines().any(|line| {
            matches!(
                line,
                "ActiveState=active"
                    | "ActiveState=activating"
                    | "ActiveState=deactivating"
                    | "ActiveState=reloading"
            )
        }));
    }
    let r = launch(&["print", &paths.target()]).await?;
    if r.status.success() {
        return Ok(true);
    }
    if r.status.code() == Some(113) {
        return Ok(false);
    }
    Err(Error::new(
        "SERVICE_MANAGER",
        "cannot inspect this login session; run from your logged-in macOS user",
    ))
}
async fn job_pid(paths: &Paths) -> Result<Option<u32>> {
    if cfg!(target_os = "linux") {
        let r = launch(&["show", "--property=MainPID", "--value", &paths.target()]).await?;
        return Ok(String::from_utf8_lossy(&r.stdout)
            .trim()
            .parse::<u32>()
            .ok()
            .filter(|pid| *pid > 0));
    }
    let r = launch(&["print", &paths.target()]).await?;
    if !r.status.success() {
        return Ok(None);
    }
    Ok(String::from_utf8_lossy(&r.stdout).lines().find_map(|line| {
        line.trim()
            .strip_prefix("pid = ")
            .and_then(|value| value.parse().ok())
    }))
}
fn checked(result: std::process::Output) -> Result<()> {
    if result.status.success() {
        Ok(())
    } else {
        Err(Error::new(
            "SERVICE_MANAGER",
            format!(
                "user service manager failed ({}); use service status or foreground serve for diagnosis",
                result.status
            ),
        ))
    }
}
pub async fn install(profile: &Path, origin: &str, login: bool) -> Result<()> {
    supported()?;
    Identity::load(profile)?;
    crate::pairing::Store::open(profile, origin)?;
    let paths = Paths::new(profile)?;
    if loaded(&paths).await? {
        return Err(Error::new(
            "SERVICE_RUNNING",
            "stop this profile's service before installing or upgrading",
        ));
    }
    private_directory(&paths.directory, true)?;
    let parent = paths.definition.parent().unwrap();
    user_directory(parent, true)?;
    let contents = definition(
        &paths,
        &fs::canonicalize(profile).map_err(io)?,
        origin,
        login,
    )?;
    let binary = fs::read(std::env::current_exe().map_err(io)?).map_err(io)?;
    atomic(&paths.binary, &binary, 0o700)?;
    atomic(&paths.definition, contents.as_bytes(), 0o600)?;
    atomic(
        &paths.config,
        &serde_json::to_vec(&Config {
            version: 1,
            origin: origin.to_owned(),
            start_at_login: login,
        })
        .map_err(|_| Error::new("SERVICE_CONFIG", "cannot encode service configuration"))?,
        0o600,
    )?;
    if cfg!(target_os = "linux") {
        checked(launch(&["daemon-reload"]).await?)?;
        checked(launch(&[if login { "enable" } else { "disable" }, &paths.target()]).await?)?;
    }
    println!(
        "Installed {}. Start with service start. Start at login: {login}.",
        paths.label
    );
    Ok(())
}
pub async fn start(profile: &Path) -> Result<()> {
    supported()?;
    let paths = Paths::new(profile)?;
    if !paths.installed()? {
        return Err(Error::new(
            "SERVICE_NOT_INSTALLED",
            "run service install first",
        ));
    }
    if !loaded(&paths).await? {
        if crate::service::status(profile).await.is_ok() {
            return Err(Error::new(
                "SERVICE_BUSY",
                "stop this profile's foreground daemon before starting its user service",
            ));
        }
        if !cfg!(target_os = "linux") {
            checked(
                launch(&[
                    "bootstrap",
                    &format!("gui/{}", unsafe { libc::geteuid() }),
                    paths.definition.to_str().unwrap(),
                ])
                .await?,
            )?;
        }
    }
    checked(
        launch(&[
            if cfg!(target_os = "linux") {
                "start"
            } else {
                "kickstart"
            },
            &paths.target(),
        ])
        .await?,
    )?;
    for _ in 0..30 {
        if let Ok((origin, online, pid)) = crate::service::status(profile).await
            && job_pid(&paths).await? == Some(pid)
        {
            println!(
                "Running ({origin}); Relay {}.",
                if online { "online" } else { "connecting" }
            );
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err(Error::new(
        "SERVICE_START_FAILED",
        "job loaded but local service did not become ready; stop it and run foreground serve to inspect the error",
    ))
}
pub async fn stop(profile: &Path) -> Result<()> {
    supported()?;
    let paths = Paths::new(profile)?;
    if loaded(&paths).await? {
        checked(
            launch(&[
                if cfg!(target_os = "linux") {
                    "stop"
                } else {
                    "bootout"
                },
                &paths.target(),
            ])
            .await?,
        )?;
    }
    println!("Stopped. Identity, grants, targets and Herdr processes are preserved.");
    Ok(())
}
pub async fn status(profile: &Path) -> Result<()> {
    supported()?;
    let paths = Paths::new(profile)?;
    if cfg!(target_os = "linux") {
        let output = tokio::process::Command::new("/usr/bin/loginctl")
            .args([
                "show-user",
                &unsafe { libc::geteuid() }.to_string(),
                "--property=Linger",
                "--value",
            ])
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        match tokio::time::timeout(std::time::Duration::from_secs(3), output).await {
            Ok(Ok(result)) if result.status.success() => println!(
                "Linger: {}. Login/session policy still controls user-service availability.",
                String::from_utf8_lossy(&result.stdout).trim()
            ),
            _ => println!(
                "Linger: unknown. User-service availability after logout is not confirmed."
            ),
        }
    }
    if !paths.installed()? {
        println!("Not installed.");
        return Ok(());
    }
    if !loaded(&paths).await? {
        println!("Installed, stopped.");
        return Ok(());
    }
    match crate::service::status(profile).await {
        Ok((origin, online, pid)) if job_pid(&paths).await? == Some(pid) => println!(
            "Running ({origin}); Relay {}.",
            if online { "online" } else { "connecting" }
        ),
        _ => println!(
            "Loaded, but daemon is not responding. Stop it and run foreground serve for diagnosis."
        ),
    }
    Ok(())
}
pub async fn uninstall(profile: &Path) -> Result<()> {
    supported()?;
    let paths = Paths::new(profile)?;
    stop(profile).await?;
    if cfg!(target_os = "linux") && exists(&paths.definition)? {
        checked(launch(&["disable", &paths.target()]).await?)?;
    }
    for path in [&paths.definition, &paths.binary, &paths.config] {
        if exists(path)? {
            safe_file(path)?;
            fs::remove_file(path).map_err(io)?;
        }
    }
    if cfg!(target_os = "linux") {
        checked(launch(&["daemon-reload"]).await?)?;
    }
    println!("Uninstalled user service. Identity, grants and shared targets are preserved.");
    Ok(())
}
fn io(error: std::io::Error) -> Error {
    Error::new("SERVICE_IO", error.to_string())
}
