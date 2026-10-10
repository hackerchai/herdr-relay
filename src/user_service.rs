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
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        io::AsRawFd,
    },
    path::{Path, PathBuf},
    process::Stdio,
};

macro_rules! service_notice {
    ($($arg:tt)*) => { if !crate::tui::is_active() { println!($($arg)*); } };
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    version: u8,
    pub origin: String,
    pub start_at_login: bool,
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
// Serialize mutations from separate CLI installations as well as pending starts.
fn mutation_lock(profile: &Path) -> Result<fs::File> {
    let paths = Paths::new(profile)?;
    private_directory(&paths.directory, true)?;
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(paths.directory.join("operation.lock"))
        .map_err(io)?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(Error::new(
            "SERVICE_BUSY",
            "another service operation is in progress; retry when it finishes",
        ));
    }
    Ok(lock)
}
fn clear_update_files(paths: &Paths) -> Result<()> {
    for name in [
        "pending-herdr-relay",
        "previous-herdr-relay",
        "previous-definition",
    ] {
        let path = paths.directory.join(name);
        if exists(&path)? {
            safe_file(&path)?;
            fs::remove_file(path).map_err(io)?;
        }
    }
    Ok(())
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
        Some("daemon"),
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
        Some("daemon"),
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
    let _lock = mutation_lock(profile)?;
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
    clear_update_files(&paths)?;
    service_notice!(
        "Installed {}. Start with service start. Start at login: {login}.",
        paths.label
    );
    Ok(())
}
pub fn configuration(profile: &Path) -> Result<Option<Config>> {
    if !profile.exists() {
        return Ok(None);
    }
    let paths = Paths::new(profile)?;
    if !paths.installed()? {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    safe_file(&paths.config)?
        .take(8193)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| Error::new("SERVICE_CONFIG", "invalid installed service configuration"))
}

/// A package-manager CLI upgrade does not replace the separately installed daemon.
pub async fn needs_upgrade(profile: &Path, version: &str) -> Result<bool> {
    if configuration(profile)?.is_none() {
        return Ok(false);
    }
    let paths = Paths::new(profile)?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(&paths.binary)
            .arg("--version")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await;
    Ok(!matches!(result, Ok(Ok(output)) if output.status.success()
        && String::from_utf8_lossy(&output.stdout).trim() == format!("herdr-relay {version}")))
}

pub async fn set_autostart(profile: &Path, enabled: bool) -> Result<()> {
    let _lock = mutation_lock(profile)?;
    let mut config = configuration(profile)?.ok_or_else(|| {
        Error::new(
            "SERVICE_NOT_INSTALLED",
            "run serve or service install first",
        )
    })?;
    let paths = Paths::new(profile)?;
    // Preserve the installed daemon invocation during a settings-only change:
    // an older copied executable may not yet support the new daemon command.
    if cfg!(target_os = "macos") {
        let mut contents = String::new();
        safe_file(&paths.definition)?
            .take(65537)
            .read_to_string(&mut contents)
            .map_err(io)?;
        let contents = contents.replace(
            "<key>RunAtLoad</key><true/>",
            "<key>RunAtLoad</key><false/>",
        );
        let contents = if enabled {
            contents.replace(
                "<key>RunAtLoad</key><false/>",
                "<key>RunAtLoad</key><true/>",
            )
        } else {
            contents
        };
        atomic(&paths.definition, contents.as_bytes(), 0o600)?;
    }
    if cfg!(target_os = "linux") {
        checked(launch(&["daemon-reload"]).await?)?;
        checked(launch(&[if enabled { "enable" } else { "disable" }, &paths.target()]).await?)?;
    }
    config.start_at_login = enabled;
    atomic(
        &paths.config,
        &serde_json::to_vec(&config)
            .map_err(|_| Error::new("SERVICE_CONFIG", "cannot encode service configuration"))?,
        0o600,
    )?;
    Ok(())
}

pub async fn ensure(profile: &Path, origin: &str, autostart: bool) -> Result<()> {
    match configuration(profile)? {
        Some(config) => {
            if config.origin != origin {
                return Err(Error::new(
                    "ORIGIN_MISMATCH",
                    "installed service uses a different Relay; switch the Relay explicitly first",
                ));
            }
            if config.start_at_login != autostart {
                set_autostart(profile, autostart).await?;
            }
        }
        None => install(profile, origin, autostart).await?,
    }
    if crate::service::status(profile).await.is_err() {
        start(profile).await?;
    }
    Ok(())
}

/// Linger is a user-wide policy and is never disabled by service uninstall.
pub async fn enable_boot_start(profile: &Path) -> Result<()> {
    if !cfg!(target_os = "linux") {
        return Err(Error::new(
            "SERVICE_UNSUPPORTED",
            "macOS autostart runs at login; boot-before-login is not supported",
        ));
    }
    set_autostart(profile, true).await?;
    // Enabling an already-enabled user policy must not require fresh polkit authorization.
    let current = tokio::process::Command::new("/usr/bin/loginctl")
        .args([
            "show-user",
            &unsafe { libc::geteuid() }.to_string(),
            "--property=Linger",
            "--value",
        ])
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    if let Ok(Ok(output)) = tokio::time::timeout(std::time::Duration::from_secs(3), current).await
        && output.status.success()
        && output.stdout.strip_suffix(b"\n").unwrap_or(&output.stdout) == b"yes"
    {
        return Ok(());
    }
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::new("/usr/bin/loginctl")
            .args(["enable-linger", &unsafe { libc::geteuid() }.to_string()])
            .kill_on_drop(true)
            .status(),
    )
    .await
    .map_err(|_| Error::new("SERVICE_MANAGER", "enabling boot startup timed out"))?
    .map_err(io)?;
    if !result.success() {
        return Err(Error::new(
            "SERVICE_MANAGER",
            "login startup is enabled, but boot/after-logout startup requires loginctl enable-linger authorization",
        ));
    }
    Ok(())
}
pub async fn start(profile: &Path) -> Result<()> {
    let _lock = mutation_lock(profile)?;
    start_with_pending(profile).await
}
async fn reload_manager() -> Result<()> {
    if cfg!(target_os = "linux") {
        checked(launch(&["daemon-reload"]).await?)?;
    }
    Ok(())
}
async fn restore_service(profile: &Path, paths: &Paths, was_running: bool) -> Result<()> {
    let _ = stop_inner(profile).await;
    restore_previous(paths)?;
    reload_manager().await?;
    if was_running {
        start_inner(profile).await.map_err(|_| {
            Error::new(
                "UPDATE_ROLLBACK",
                "previous service restored but could not start; run service status",
            )
        })?;
    }
    Ok(())
}
async fn start_with_pending(profile: &Path) -> Result<()> {
    let paths = Paths::new(profile)?;
    let pending =
        exists(&paths.directory.join("pending-herdr-relay"))? && job_pid(&paths).await?.is_none();
    if !pending {
        return start_inner(profile).await;
    }
    // launchd can retain an inactive job's old command; unload it before changing it.
    stop_inner(profile).await?;
    activate_pending(profile)?;
    let result = async {
        reload_manager().await?;
        start_inner(profile).await
    }
    .await;
    if let Err(error) = result {
        restore_service(profile, &paths, true).await?;
        return Err(Error::new(
            "UPDATE_ACTIVATION",
            format!(
                "pending update failed ({}); previous service restored",
                error.code
            ),
        ));
    }
    Ok(())
}
fn restore_previous(paths: &Paths) -> Result<()> {
    atomic(
        &paths.binary,
        &fs::read(paths.directory.join("previous-herdr-relay")).map_err(io)?,
        0o700,
    )?;
    atomic(
        &paths.definition,
        &fs::read(paths.directory.join("previous-definition")).map_err(io)?,
        0o600,
    )
}

async fn start_inner(profile: &Path) -> Result<()> {
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
            service_notice!(
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
    let _lock = mutation_lock(profile)?;
    stop_inner(profile).await
}
async fn stop_inner(profile: &Path) -> Result<()> {
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
    service_notice!("Stopped. Identity, grants, targets and Herdr processes are preserved.");
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
            Ok(Ok(result)) if result.status.success() => service_notice!(
                "Linger: {}. Login/session policy still controls user-service availability.",
                String::from_utf8_lossy(&result.stdout).trim()
            ),
            _ => service_notice!(
                "Linger: unknown. User-service availability after logout is not confirmed."
            ),
        }
    }
    if !paths.installed()? {
        service_notice!("Not installed.");
        return Ok(());
    }
    if let Some(config) = configuration(profile)? {
        service_notice!("Start at login: {}.", config.start_at_login);
    }
    if exists(&paths.directory.join("pending-herdr-relay"))? {
        service_notice!("Service update pending. Apply with service stop, then service start.");
    }
    if !loaded(&paths).await? {
        service_notice!("Installed, stopped.");
        return Ok(());
    }
    match crate::service::status(profile).await {
        Ok((origin, online, pid)) if job_pid(&paths).await? == Some(pid) => service_notice!(
            "Running ({origin}); Relay {}.",
            if online { "online" } else { "connecting" }
        ),
        _ => service_notice!(
            "Loaded, but daemon is not responding. Stop it and run foreground serve for diagnosis."
        ),
    }
    Ok(())
}
pub async fn uninstall(profile: &Path) -> Result<()> {
    supported()?;
    let _lock = mutation_lock(profile)?;
    let paths = Paths::new(profile)?;
    stop_inner(profile).await?;
    clear_update_files(&paths)?;
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
    service_notice!("Uninstalled user service. Identity, grants and shared targets are preserved.");
    Ok(())
}
fn io(error: std::io::Error) -> Error {
    Error::new("SERVICE_IO", error.to_string())
}

fn activate_pending(profile: &Path) -> Result<()> {
    let paths = Paths::new(profile)?;
    let pending = paths.directory.join("pending-herdr-relay");
    if !exists(&pending)? {
        return Ok(());
    }
    let config = configuration(profile)?
        .ok_or_else(|| Error::new("SERVICE_CONFIG", "missing service configuration"))?;
    activate_files(&paths, profile, &config)
}
fn activate_files(paths: &Paths, profile: &Path, config: &Config) -> Result<()> {
    let pending = paths.directory.join("pending-herdr-relay");
    let mut bytes = Vec::new();
    safe_file(&pending)?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(Error::new(
            "SERVICE_CONFIG",
            "pending executable exceeds size limit",
        ));
    }
    let old = fs::read(&paths.binary).map_err(io)?;
    let old_definition = fs::read(&paths.definition).map_err(io)?;
    atomic(&paths.directory.join("previous-herdr-relay"), &old, 0o700)?;
    atomic(
        &paths.directory.join("previous-definition"),
        &old_definition,
        0o600,
    )?;
    let activation = (|| {
        atomic(&paths.binary, &bytes, 0o700)?;
        atomic(
            &paths.definition,
            definition(
                paths,
                &fs::canonicalize(profile).map_err(io)?,
                &config.origin,
                config.start_at_login,
            )?
            .as_bytes(),
            0o600,
        )?;
        fs::remove_file(&pending).map_err(io)?;
        Ok(())
    })();
    if let Err(error) = activation {
        restore_previous(paths)?;
        // A failed candidate must not be retried implicitly by service start.
        if exists(&pending)? {
            fs::remove_file(&pending).map_err(io)?;
        }
        return Err(error);
    }
    Ok(())
}

pub async fn upgrade(profile: &Path, bytes: &[u8], force_restart: bool) -> Result<&'static str> {
    if configuration(profile)?.is_none() {
        return Ok("No background service is installed.");
    }
    let _lock = mutation_lock(profile)?;
    let paths = Paths::new(profile)?;
    atomic(&paths.directory.join("pending-herdr-relay"), bytes, 0o700)?;
    let running = job_pid(&paths).await?.is_some();
    // Hold every native reservation until the daemon has stopped. This closes
    // the check-to-stop race without disconnecting an already attached client.
    let mut reservations = Vec::new();
    if running && !force_restart {
        for target in crate::targets::Store::open(profile)?
            .list()?
            .into_iter()
            .filter(|t| t.enabled)
        {
            if let Ok(native) = target.locator.resolve() {
                match native.reserve() {
                    Ok(lock) => reservations.push(lock),
                    Err(_) => {
                        return Ok(
                            "CLI updated; service update pending because a session is active. Apply later with service stop && service start, or update --restart-service.",
                        );
                    }
                }
            }
        }
    }
    stop_inner(profile).await?;
    drop(reservations);
    if let Err(error) = activate_pending(profile) {
        if running {
            start_inner(profile).await?;
        }
        return Err(error);
    }
    let activation = async {
        reload_manager().await?;
        if running {
            start_inner(profile).await?;
        }
        Ok::<_, Error>(())
    }
    .await;
    if let Err(error) = activation {
        restore_service(profile, &paths, running).await?;
        return Err(Error::new(
            "UPDATE_ACTIVATION",
            format!(
                "new service failed ({}); previous service restored",
                error.code
            ),
        ));
    }
    Ok(if running {
        "Background service upgraded and restarted."
    } else {
        "Background service upgraded; it remains stopped."
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn service_activation_rolls_back_both_files_and_discards_failed_candidate() {
        let root = tempfile::tempdir().unwrap();
        let paths = Paths {
            directory: root.path().to_owned(),
            binary: root.path().join("herdr-relay"),
            config: root.path().join("config.json"),
            definition: root.path().join("service-definition"),
            label: "test".into(),
        };
        let config = Config {
            version: 1,
            origin: "https://relay.example".into(),
            start_at_login: true,
        };
        atomic(&paths.binary, b"old executable", 0o700).unwrap();
        atomic(&paths.definition, b"old serve --relay definition", 0o600).unwrap();
        let pending = paths.directory.join("pending-herdr-relay");
        atomic(&pending, b"new executable", 0o700).unwrap();
        activate_files(&paths, root.path(), &config).unwrap();
        assert_eq!(fs::read(&paths.binary).unwrap(), b"new executable");
        let stable = fs::read(&paths.definition).unwrap();
        assert!(String::from_utf8_lossy(&stable).contains("daemon"));
        assert!(!pending.exists());
        restore_previous(&paths).unwrap();
        assert_eq!(fs::read(&paths.binary).unwrap(), b"old executable");
        assert_eq!(
            fs::read(&paths.definition).unwrap(),
            b"old serve --relay definition"
        );
        atomic(&pending, b"bad candidate", 0o700).unwrap();
        let invalid = Config {
            origin: "invalid\norigin".into(),
            ..config
        };
        assert_eq!(
            activate_files(&paths, root.path(), &invalid)
                .unwrap_err()
                .code,
            "SERVICE_PATH"
        );
        assert_eq!(fs::read(&paths.binary).unwrap(), b"old executable");
        assert_eq!(
            fs::read(&paths.definition).unwrap(),
            b"old serve --relay definition"
        );
        assert!(!pending.exists());
    }
}
