use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};
use herdr_relay::{Error, Result, native::NativeTarget};
use std::path::PathBuf;

const DEFAULT_RELAY_ORIGIN: &str = "https://herdr-relay.hackerchai.com";

#[cfg(test)]
mod cli_tests {
    #[test]
    fn interactive_defaults_and_unattended_service_parse_separately() {
        use super::{Action, Cli};
        use clap::Parser;
        assert!(
            Cli::try_parse_from(["herdr-relay"])
                .unwrap()
                .command
                .is_none()
        );
        assert!(matches!(
            Cli::try_parse_from(["herdr-relay", "pair"])
                .unwrap()
                .command,
            Some(Action::Pair { action: None })
        ));
        assert!(matches!(
            Cli::try_parse_from(["herdr-relay", "connect"])
                .unwrap()
                .command,
            Some(Action::Connect {
                host: None,
                target: None,
                ..
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["herdr-relay", "serve"])
                .unwrap()
                .command,
            Some(Action::Serve {
                foreground: false,
                relay: false,
                ..
            })
        ));
        assert!(matches!(
            Cli::try_parse_from(["herdr-relay", "daemon"])
                .unwrap()
                .command,
            Some(Action::Daemon { .. })
        ));
        assert!(matches!(
            Cli::try_parse_from(["herdr-relay", "serve", "--relay"])
                .unwrap()
                .command,
            Some(Action::Serve { relay: true, .. })
        ));
    }

    use super::*;

    /// Serializes tests that read or change process environment variables.
    pub(super) static ENVIRONMENT: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn hosted_origin_is_the_default_and_self_hosting_stays_explicit() {
        let _environment = ENVIRONMENT.lock().unwrap_or_else(|e| e.into_inner());
        let cli = Cli::try_parse_from(["herdr-relay", "serve", "--relay"]).unwrap();
        let Some(Action::Serve { origin, .. }) = cli.command else {
            panic!("serve command")
        };
        assert_eq!(origin, "https://herdr-relay.hackerchai.com");
        let cli = Cli::try_parse_from([
            "herdr-relay",
            "connect",
            "--host",
            "host",
            "--target",
            "target",
            "--origin",
            "https://example.com",
        ])
        .unwrap();
        let Some(Action::Connect { origin, .. }) = cli.command else {
            panic!("connect command")
        };
        assert_eq!(origin, "https://example.com");
    }
}

#[derive(Parser)]
#[command(
    version,
    about = "Share and connect to Herdr sessions by name",
    after_help = "Quick start:\n  Server: herdr-relay serve\n  Client: herdr-relay pair (paste a link), then herdr-relay\n  Maintenance: herdr-relay service --help\n\nIn menus: arrows / j,k move; Space selects; Enter applies; ? / F1 opens help; Esc returns."
)]
struct Cli {
    /// Private Relay state directory (separate from Herdr configuration).
    #[arg(long, global = true, env = "HERDR_RELAY_PROFILE")]
    profile: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Action>,
}

#[derive(Subcommand)]
enum Action {
    /// Update to the latest stable release (download site, then GitHub Releases).
    #[command(display_order = 40)]
    Update {
        /// Report the available version without installing it.
        #[arg(long)]
        check: bool,
        /// Restart the server even if clients are attached; also apply pending updates.
        #[arg(long)]
        restart_service: bool,
        /// GitHub owner/repository hosting release assets.
        #[arg(long, env = "HERDR_RELAY_UPDATE_REPOSITORY")]
        repository: Option<String>,
        /// Release download site tried before GitHub Releases (exact https:// origin).
        #[arg(long, env = "HERDR_RELAY_UPDATE_URL")]
        download_url: Option<String>,
    },
    /// Run the Relay service without a terminal (used by service managers).
    #[command(hide = true)]
    Daemon {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
        #[arg(long, hide = true)]
        relay_local_url: Option<String>,
    },
    /// List or remove hosts this device can access.
    #[command(display_order = 35)]
    Hosts {
        #[command(subcommand)]
        action: HostsAction,
    },
    /// Save settings in config.toml (flags and environment variables still win).
    #[command(display_order = 45)]
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Change the Relay trust scope while preserving identity and shared targets.
    #[command(hide = true)]
    Relay {
        #[command(subcommand)]
        action: RelayAction,
    },
    /// Manage this profile's background user service (never starts Herdr).
    #[command(display_order = 50)]
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Select Herdr sessions to share and start the background service with sleep protection.
    #[command(display_order = 10, after_help = herdr_relay::power::HELP)]
    Serve {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
        /// Run without menus in the foreground (local diagnostics).
        #[arg(long)]
        foreground: bool,
        /// Compatibility: run the publishing daemon without menus.
        #[arg(long)]
        relay: bool,
        /// Explicit loopback HTTP transport for local Workers development only.
        #[arg(long, requires = "relay")]
        relay_local_url: Option<String>,
        #[arg(long, conflicts_with = "socket", requires = "relay")]
        session: Option<String>,
        #[arg(long, requires = "relay")]
        socket: Option<PathBuf>,
        #[arg(long)]
        herdr_config_dir: Option<PathBuf>,
        /// Name is sent only inside the device-encrypted channel.
        #[arg(long)]
        name: Option<String>,
    },
    /// List shared targets and their encrypted status on an authorized host.
    #[command(alias = "status")]
    #[command(hide = true)]
    Targets {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
        #[arg(long)]
        host: String,
        #[arg(long)]
        relay_local_url: Option<String>,
    },
    /// Choose a host and Herdr session in the terminal menu.
    #[command(display_order = 30)]
    Connect {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        target: Option<String>,
        #[arg(long, env = "HERDR_RELAY_HERDR", default_value = "herdr")]
        herdr: PathBuf,
        #[arg(long)]
        relay_local_url: Option<String>,
    },
    /// Add or manage persistent, explicitly shared local targets.
    #[command(hide = true)]
    Share {
        #[command(subcommand)]
        action: ShareAction,
    },
    /// Paste a pairing link to add a host and choose its session.
    #[command(display_order = 20)]
    Pair {
        #[command(subcommand)]
        action: Option<PairAction>,
    },
    /// Show independent incoming and outgoing device grants.
    #[command(hide = true)]
    Devices {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
    },
    /// Manage incoming device authorization; reverse grants remain independent.
    #[command(hide = true)]
    Peers {
        #[command(subcommand)]
        action: PeerAction,
    },
    /// Initialize, inspect, or locally diagnose a persistent device identity.
    #[command(hide = true)]
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },
    /// Connect a native Herdr client through a local byte-stream bridge.
    #[command(hide = true)]
    Local {
        #[arg(long, required_unless_present = "socket", conflicts_with = "socket")]
        session: Option<String>,
        /// An explicit existing native client socket (never the JSON API socket).
        #[arg(long)]
        socket: Option<PathBuf>,
        /// Herdr's configuration directory, normally $XDG_CONFIG_HOME/herdr.
        #[arg(long, requires = "session")]
        herdr_config_dir: Option<PathBuf>,
        #[arg(long, env = "HERDR_RELAY_HERDR", default_value = "herdr")]
        herdr: PathBuf,
    },
}

#[derive(Subcommand)]
enum HostsAction {
    /// Show paired hosts with their aliases and device IDs.
    List {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
    },
    /// Forget a host by alias, name or device ID. Pairing again needs a new link.
    Remove {
        host: String,
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show the configuration file and its settings; tokens stay hidden.
    Show,
    /// Save a setting: origin, profile, herdr, update-check, update-url,
    /// update-repository, direct (on/off), direct-port or token. Omit a token's
    /// value to enter it hidden.
    Set {
        key: String,
        value: Option<String>,
        /// The self-hosted relay a token belongs to (default: the configured origin).
        #[arg(long)]
        origin: Option<String>,
    },
    /// Remove a setting so its environment variable or default applies.
    Unset {
        key: String,
        #[arg(long)]
        origin: Option<String>,
    },
}

#[derive(Subcommand)]
enum RelayAction {
    /// Read aggregate capacity counters using an operator-authorized device identity.
    Status {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
        #[arg(long, hide = true)]
        dev_relay_url: Option<String>,
    },
    /// Stop the daemon first. All pairing invitations and grants are invalidated.
    Switch {
        #[arg(long)]
        from: String,
        #[arg(long)]
        to: String,
    },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Copy this CLI to persistent private storage and install its service definition.
    Install {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
        /// Opt in to automatic start in your login session.
        #[arg(long)]
        start_at_login: bool,
    },
    /// Configure server autostart; boot mode is available on Linux.
    Autostart {
        #[arg(value_enum)]
        mode: AutostartMode,
    },
    Start,
    Stop,
    Status,
    /// Remove the service; preserve identity, grants and shared targets.
    Uninstall,
}

#[derive(clap::ValueEnum, Clone)]
enum AutostartMode {
    On,
    Off,
    Boot,
}

#[derive(Subcommand)]
enum PeerAction {
    List {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
    },
    Revoke {
        device: String,
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
    },
}

#[derive(Subcommand)]
enum ShareAction {
    Add {
        #[arg(long)]
        name: String,
        #[arg(long, required_unless_present = "socket", conflicts_with = "socket")]
        session: Option<String>,
        #[arg(long)]
        socket: Option<PathBuf>,
        #[arg(long, requires = "session")]
        herdr_config_dir: Option<PathBuf>,
    },
    List,
    Enable {
        target: String,
    },
    Disable {
        target: String,
    },
    Remove {
        target: String,
    },
}

#[derive(Subcommand)]
enum PairAction {
    Create,
    Accept {
        /// Local diagnostic transport to the publishing profile's daemon.sock.
        #[arg(long, conflicts_with = "relay_local_url")]
        local_socket: Option<PathBuf>,
        /// Explicit loopback HTTP transport for local Workers development only.
        #[arg(long)]
        relay_local_url: Option<String>,
    },
    Revoke {
        invitation_id: String,
    },
}

#[derive(Subcommand)]
enum IdentityAction {
    Init,
    Show,
    /// Authenticate two profiles over a local-only Unix socket pair; never dials a network service.
    Diagnose {
        #[arg(long)]
        peer_profile: PathBuf,
        #[arg(long, default_value = "https://diagnostic.invalid")]
        origin: String,
    },
}

#[tokio::main]
async fn main() {
    let config =
        match herdr_relay::config::path().and_then(|p| herdr_relay::config::Config::load(&p)) {
            Ok(config) => config,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        };
    let cli = try_parse_with(std::env::args_os(), &config).unwrap_or_else(|e| e.exit());
    herdr_relay::config::use_config(config);
    if let Err(error) = run(cli).await {
        if error.code == "BACK" {
            return;
        }
        eprintln!("{error}");
        std::process::exit(if error.code == "CANCELLED" { 130 } else { 1 });
    }
}

/// Parse arguments with `config.toml` values as defaults below flags and
/// environment variables. Arguments are matched by their environment variable.
fn try_parse_with<I, T>(
    args: I,
    config: &herdr_relay::config::Config,
) -> std::result::Result<Cli, clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    fn defaults(command: clap::Command, values: &[(&str, String)]) -> clap::Command {
        command
            .mut_args(|arg| {
                let value = arg.get_env().and_then(|env| {
                    values
                        .iter()
                        .find(|(name, _)| env == std::ffi::OsStr::new(name))
                });
                match value {
                    Some((_, value)) => arg.default_value(value.clone()),
                    None => arg,
                }
            })
            .mut_subcommands(|sub| defaults(sub, values))
    }
    let values: Vec<(&str, String)> = [
        ("HERDR_RELAY_ORIGIN", config.origin.clone()),
        (
            "HERDR_RELAY_PROFILE",
            config.profile.as_ref().map(|p| p.display().to_string()),
        ),
        (
            "HERDR_RELAY_HERDR",
            config.herdr.as_ref().map(|p| p.display().to_string()),
        ),
        (
            "HERDR_RELAY_UPDATE_REPOSITORY",
            config.update_repository.clone(),
        ),
        ("HERDR_RELAY_UPDATE_URL", config.update_url.clone()),
    ]
    .into_iter()
    .filter_map(|(name, value)| Some((name, value?)))
    .collect();
    let matches = defaults(Cli::command(), &values).try_get_matches_from(args)?;
    Cli::from_arg_matches(&matches)
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// Plain `herdr-relay` opens the host menu, honoring the environment and config.
fn default_action(config: &herdr_relay::config::Config) -> Action {
    Action::Connect {
        origin: env_var("HERDR_RELAY_ORIGIN")
            .or_else(|| config.origin.clone())
            .unwrap_or_else(|| DEFAULT_RELAY_ORIGIN.into()),
        host: None,
        target: None,
        herdr: herdr_path(config),
        relay_local_url: None,
    }
}

fn herdr_path(config: &herdr_relay::config::Config) -> PathBuf {
    env_var("HERDR_RELAY_HERDR")
        .map(PathBuf::from)
        .or_else(|| config.herdr.clone())
        .unwrap_or_else(|| "herdr".into())
}

async fn run(cli: Cli) -> Result<()> {
    let config = herdr_relay::config::current();
    match cli.command.unwrap_or_else(|| default_action(&config)) {
        Action::Config { action } => configure(action, &config),
        Action::Hosts { action } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            match action {
                HostsAction::List { origin } => {
                    let origin = paired_origin(&profile, &origin)?;
                    let hosts = herdr_relay::hosts::list(&profile, &origin)?;
                    if hosts.is_empty() {
                        println!("No paired hosts. Run herdr-relay pair to add one.");
                    }
                    for host in hosts {
                        let name = if host.name == host.alias {
                            String::new()
                        } else {
                            format!("  ({})", herdr_relay::names::display(&host.name))
                        };
                        println!(
                            "{}  {}{name}",
                            host.identity,
                            herdr_relay::names::display(&host.alias)
                        );
                    }
                }
                HostsAction::Remove { host, origin } => {
                    let origin = paired_origin(&profile, &origin)?;
                    let removed = herdr_relay::hosts::remove(&profile, &origin, &host)?;
                    println!(
                        "Removed {}. Pair again with a new link to regain access; the host can revoke this device with peers revoke.",
                        herdr_relay::names::display(&removed.alias)
                    );
                }
            }
            Ok(())
        }
        Action::Update {
            check,
            restart_service,
            repository,
            download_url,
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            herdr_relay::update::run(
                &profile,
                check,
                restart_service,
                repository.as_deref(),
                download_url.as_deref(),
            )
            .await
        }
        Action::Daemon {
            origin,
            relay_local_url,
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            let remote =
                herdr_relay::relay::Relay::new(&profile, &origin, relay_local_url.as_deref())?;
            herdr_relay::service::run(&profile, &origin, Some(remote)).await
        }
        Action::Relay {
            action:
                RelayAction::Status {
                    origin,
                    dev_relay_url,
                },
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            let relay =
                herdr_relay::relay::Relay::new(&profile, &origin, dev_relay_url.as_deref())?;
            println!("{}", relay.operator_status().await?);
            Ok(())
        }
        Action::Relay {
            action: RelayAction::Switch { from, to },
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            herdr_relay::pairing::Store::switch_origin(&profile, &from, &to)?;
            if from == to {
                println!("Relay origin unchanged: {to}.");
            } else {
                println!(
                    "Relay origin: {to}. Identity and shared targets preserved; pair again for access. Reinstall any user service with this origin before starting it."
                );
            }
            Ok(())
        }
        Action::Service { action } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            use herdr_relay::user_service as managed;
            match action {
                ServiceAction::Install {
                    origin,
                    start_at_login,
                } => managed::install(&profile, &origin, start_at_login).await,
                ServiceAction::Autostart { mode } => {
                    match mode {
                        AutostartMode::On => managed::set_autostart(&profile, true).await?,
                        AutostartMode::Off => managed::set_autostart(&profile, false).await?,
                        AutostartMode::Boot => managed::enable_boot_start(&profile).await?,
                    }
                    managed::status(&profile).await
                }
                ServiceAction::Start => managed::start(&profile).await,
                ServiceAction::Stop => managed::stop(&profile).await,
                ServiceAction::Status => managed::status(&profile).await,
                ServiceAction::Uninstall => managed::uninstall(&profile).await,
            }
        }
        Action::Serve {
            origin,
            foreground,
            relay,
            relay_local_url,
            session,
            socket,
            herdr_config_dir,
            name,
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            if !foreground && !relay {
                return herdr_relay::interactive::serve(
                    &profile,
                    &origin,
                    &herdr_config_dir.map(Ok).unwrap_or_else(config_dir)?,
                )
                .await;
            }
            let remote = if relay {
                Some(herdr_relay::relay::Relay::new(
                    &profile,
                    &origin,
                    relay_local_url.as_deref(),
                )?)
            } else {
                None
            };
            if session.is_some() || socket.is_some() {
                let display =
                    name.unwrap_or_else(|| session.clone().unwrap_or_else(|| "Herdr".into()));
                let locator = select_locator(session, socket, herdr_config_dir)?;
                let native = locator.resolve()?;
                let store = herdr_relay::targets::Store::open(&profile)?;
                let existing = store.list()?.into_iter().find(|t| {
                    t.locator
                        .resolve()
                        .is_ok_and(|candidate| candidate.same_instance(&native))
                });
                let target = if let Some(target) = existing {
                    if !target.enabled {
                        return Err(Error::new(
                            "TARGET_DISABLED",
                            "use share enable to explicitly resume this target",
                        ));
                    }
                    target
                } else {
                    store.add(display, locator)?
                };
                eprintln!("Shared target: {}", target.id);
            }
            herdr_relay::service::run(&profile, &origin, remote).await
        }
        Action::Share { action } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            let store = herdr_relay::targets::Store::open(&profile)?;
            match action {
                ShareAction::Add {
                    name,
                    session,
                    socket,
                    herdr_config_dir,
                } => {
                    let target =
                        store.add(name, select_locator(session, socket, herdr_config_dir)?)?;
                    herdr_relay::service::reload_targets(&profile).await?;
                    println!("{}  enabled  {}", target.id, target.name);
                }
                ShareAction::List => {
                    for target in store.list()? {
                        let status = if !target.enabled {
                            "disabled"
                        } else if target.locator.resolve().is_ok() {
                            "enabled"
                        } else {
                            "offline"
                        };
                        println!(
                            "{}  {}  {}  {:?}",
                            target.id, status, target.name, target.locator
                        );
                    }
                }
                ShareAction::Enable { target } => {
                    let target = store.select(&target)?;
                    store.set_enabled(&target.id, true)?;
                    herdr_relay::service::reload_targets(&profile).await?;
                    println!("Enabled {}", target.id);
                }
                ShareAction::Disable { target } => {
                    let target = store.select(&target)?;
                    store.set_enabled(&target.id, false)?;
                    herdr_relay::service::reload_targets(&profile).await?;
                    println!("Disabled {}", target.id);
                }
                ShareAction::Remove { target } => {
                    let target = store.select(&target)?;
                    store.remove(&target.id)?;
                    herdr_relay::service::reload_targets(&profile).await?;
                    println!("Removed {}", target.id);
                }
            }
            Ok(())
        }
        Action::Targets {
            origin,
            host,
            relay_local_url,
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            let relay =
                herdr_relay::relay::Relay::new(&profile, &origin, relay_local_url.as_deref())?;
            for target in relay.targets(&host).await? {
                println!("{}  {}  {}", target.id, target.status, target.name);
            }
            Ok(())
        }
        Action::Connect {
            origin,
            host,
            target,
            herdr,
            relay_local_url,
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            if host.is_none() && target.is_none() {
                return herdr_relay::interactive::connect(
                    &profile,
                    &origin,
                    &herdr,
                    None,
                    relay_local_url.as_deref(),
                )
                .await;
            }
            let (Some(host), Some(target)) = (host, target) else {
                return Err(Error::new(
                    "CONNECT_ARGUMENTS",
                    "use connect without arguments for the host/session menu, or provide both --host and --target",
                ));
            };
            herdr_relay::client::check_version(&herdr).await?;
            let relay =
                herdr_relay::relay::Relay::new(&profile, &origin, relay_local_url.as_deref())?;
            let reattach = std::env::args()
                .map(|arg| shell_quote(&arg))
                .collect::<Vec<_>>()
                .join(" ");
            herdr_relay::client::connect_remote(relay, &host, &target, &herdr, &reattach).await
        }
        Action::Devices { origin } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            print_peers(&profile, &origin)?;
            Ok(())
        }
        Action::Peers { action } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            match action {
                PeerAction::List { origin } => print_peers(&profile, &origin)?,
                PeerAction::Revoke { device, origin } => {
                    let version = herdr_relay::pairing::Store::open(&profile, &origin)?
                        .revoke_peer(&device)?;
                    herdr_relay::service::reload_authorization(&profile).await?;
                    println!(
                        "Revoked incoming access for {device} (version {version}). Reverse access remains independent."
                    );
                }
            }
            Ok(())
        }
        Action::Pair { action } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            let Some(action) = action else {
                return herdr_relay::interactive::pair(&profile, &herdr_path(&config)).await;
            };
            match action {
                PairAction::Create => {
                    let invitation = herdr_relay::service::create_invitation(&profile).await?;
                    eprintln!(
                        "One-time invitation {} expires at {} (Unix time). Grants access to all explicitly shared instances.",
                        invitation.invitation_id, invitation.expires_at
                    );
                    // The link also carries this computer's name for the client.
                    println!(
                        "{}",
                        herdr_relay::names::PairLink::encode(
                            invitation,
                            herdr_relay::names::hostname()
                        )?
                    );
                }
                PairAction::Accept {
                    local_socket,
                    relay_local_url,
                } => {
                    let link = herdr_relay::names::PairLink::decode(&read_invitation().await?)?;
                    let invitation = link.invitation;
                    herdr_relay::identity::Identity::initialize(&profile)?;
                    let receipt = if let Some(local_socket) = local_socket {
                        herdr_relay::service::accept_local(&profile, &local_socket, &invitation)
                            .await?
                    } else {
                        herdr_relay::relay::Relay::new(
                            &profile,
                            &invitation.origin,
                            relay_local_url.as_deref(),
                        )?
                        .accept(&invitation)
                        .await?
                    };
                    let host = herdr_relay::names::remember(
                        &profile,
                        &invitation.origin,
                        &receipt.publisher,
                        link.hostname.as_deref(),
                        None,
                    )?;
                    println!(
                        "Paired with {}. Run herdr-relay to select a session. Reverse access was not granted.",
                        host.alias
                    );
                }
                PairAction::Revoke { invitation_id } => {
                    herdr_relay::service::revoke_invitation(&profile, &invitation_id).await?;
                    println!("Invitation revoked.");
                }
            }
            Ok(())
        }
        Action::Identity { action } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            match action {
                IdentityAction::Init => println!(
                    "{}",
                    herdr_relay::identity::Identity::initialize(&profile)?.id
                ),
                IdentityAction::Show => {
                    println!("{}", herdr_relay::identity::Identity::load(&profile)?.id)
                }
                IdentityAction::Diagnose {
                    peer_profile,
                    origin,
                } => {
                    let local = herdr_relay::identity::Identity::load(&profile)?;
                    let peer = herdr_relay::identity::Identity::load(&peer_profile)?;
                    diagnose(&local, &peer, origin).await?;
                    println!(
                        "TLS 1.3 mutual authentication and context binding succeeded\npublisher={}\nconnector={}",
                        local.id, peer.id
                    );
                }
            }
            Ok(())
        }
        Action::Local {
            session,
            socket,
            herdr_config_dir,
            herdr,
        } => {
            let target = select_target(session, socket, herdr_config_dir)?;
            let reattach = std::env::args()
                .map(|arg| shell_quote(&arg))
                .collect::<Vec<_>>()
                .join(" ");
            herdr_relay::client::run_local(target, &herdr, &reattach).await
        }
    }
}

/// The origin a profile is paired with when the default was not overridden.
fn paired_origin(profile: &std::path::Path, origin: &str) -> Result<String> {
    Ok(if origin == DEFAULT_RELAY_ORIGIN {
        herdr_relay::pairing::Store::saved_origin(profile)?.unwrap_or_else(|| origin.to_owned())
    } else {
        origin.to_owned()
    })
}

fn configure(action: ConfigAction, current: &herdr_relay::config::Config) -> Result<()> {
    let path = herdr_relay::config::path()?;
    let mut config = herdr_relay::config::Config::load(&path)?;
    // A token belongs to the named relay, else the configured one.
    let token_origin = |origin: Option<String>| {
        origin
            .or_else(|| env_var("HERDR_RELAY_ORIGIN"))
            .or_else(|| current.origin.clone())
    };
    match action {
        ConfigAction::Show => {
            println!("{}\n{}", path.display(), config.show());
            return Ok(());
        }
        ConfigAction::Set { key, value, origin } => {
            let origin = (key == "token").then(|| token_origin(origin)).flatten();
            let value = match value {
                Some(value) => value,
                None if key == "token" => {
                    if origin.as_deref() == Some(herdr_relay::config::PUBLIC_ORIGIN) {
                        return Err(Error::new(
                            "PUBLIC_RELAY",
                            "the public relay is open to everyone and never uses a token",
                        ));
                    }
                    read_hidden_line(
                        "Enter the relay access token (input hidden):",
                        "CONFIG_INPUT",
                    )?
                }
                None => {
                    return Err(Error::new(
                        "CONFIG_INVALID",
                        format!("provide a value for {key}"),
                    ));
                }
            };
            config.set(&key, &value, origin.as_deref())?;
            config.save(&path)?;
            match origin {
                Some(origin) => {
                    println!("Saved the access token for {origin} in {}.", path.display())
                }
                None => println!("Saved {key} in {}.", path.display()),
            }
        }
        ConfigAction::Unset { key, origin } => {
            let origin = (key == "token").then(|| token_origin(origin)).flatten();
            config.unset(&key, origin.as_deref())?;
            config.save(&path)?;
            println!("Removed {key} from {}.", path.display());
        }
    }
    println!(
        "A running background service applies changes after herdr-relay service stop and start."
    );
    Ok(())
}

fn print_peers(profile: &std::path::Path, origin: &str) -> Result<()> {
    let grants = herdr_relay::pairing::Store::open(profile, origin)?.grants()?;
    println!("Allowed to access this device:");
    for grant in grants.allowed_here {
        let name = herdr_relay::names::peer_name(profile, origin, &grant.device_id)
            .map(|n| format!("  {}", herdr_relay::names::display(&n)))
            .unwrap_or_default();
        println!("  {} (grant {}){name}", grant.device_id, grant.version);
    }
    println!("Hosts this device can access:");
    for grant in grants.accessible_hosts {
        println!("  {} (grant {})", grant.device_id, grant.version);
    }
    Ok(())
}

fn select_locator(
    session: Option<String>,
    socket: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<herdr_relay::targets::Locator> {
    Ok(if let Some(path) = socket {
        herdr_relay::targets::Locator::Socket { path }
    } else {
        herdr_relay::targets::Locator::Session {
            config_dir: config.map(Ok).unwrap_or_else(config_dir)?,
            session: session.unwrap(),
        }
    })
}

fn select_target(
    session: Option<String>,
    socket: Option<PathBuf>,
    config: Option<PathBuf>,
) -> Result<NativeTarget> {
    if let Some(path) = socket {
        NativeTarget::socket(&path)
    } else {
        NativeTarget::session(
            &config.map(Ok).unwrap_or_else(config_dir)?,
            session.as_deref().unwrap(),
        )
    }
}

fn profile_dir() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(root).join("herdr-relay"));
    }
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(|root| PathBuf::from(root).join(".local/state/herdr-relay"))
        .ok_or_else(|| {
            Error::new(
                "IDENTITY_STORAGE",
                "provide --profile when HOME is unavailable",
            )
        })
}

async fn diagnose(
    local: &herdr_relay::identity::Identity,
    peer: &herdr_relay::identity::Identity,
    origin: String,
) -> Result<()> {
    use herdr_relay::{
        control::{Context, Purpose, random_id},
        secure,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let context = Context {
        version: 1,
        origin,
        publisher: local.id.clone(),
        connector: peer.id.clone(),
        purpose: Purpose::Control,
        grant_version: Some(1),
        target: None,
        channel_id: random_id()?,
        boot_id: random_id()?,
    };
    let (a, b) =
        tokio::net::UnixStream::pair().map_err(|e| Error::new("LOCAL_IO", e.to_string()))?;
    let exchange = async {
        let (client, server) = tokio::try_join!(
            secure::connect(b, peer, &local.certificate, &context),
            secure::accept(a, local, &peer.certificate, &context)
        )?;
        let mut client = client;
        let mut server = server;
        client
            .write_all(b"herdr-relay diagnostic")
            .await
            .map_err(diag_io)?;
        client.flush().await.map_err(diag_io)?;
        let mut request = [0; 22];
        server.read_exact(&mut request).await.map_err(diag_io)?;
        if &request != b"herdr-relay diagnostic" {
            return Err(Error::new(
                "DIAGNOSTIC_FAILED",
                "unexpected encrypted payload",
            ));
        }
        server.write_all(b"ok").await.map_err(diag_io)?;
        server.flush().await.map_err(diag_io)?;
        let mut reply = [0; 2];
        client.read_exact(&mut reply).await.map_err(diag_io)?;
        if &reply != b"ok" {
            return Err(Error::new(
                "DIAGNOSTIC_FAILED",
                "unexpected encrypted response",
            ));
        }
        Ok(())
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), exchange)
        .await
        .map_err(|_| {
            Error::new(
                "DIAGNOSTIC_TIMEOUT",
                "local encrypted diagnostic exceeded 30 seconds",
            )
        })?
}

fn diag_io(e: std::io::Error) -> Error {
    Error::new("DIAGNOSTIC_FAILED", e.to_string())
}

fn config_dir() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("XDG_CONFIG_HOME").filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(root).join("herdr"));
    }
    std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .map(|root| PathBuf::from(root).join(".config/herdr"))
        .ok_or_else(|| {
            Error::new(
                "INVALID_TARGET",
                "provide --herdr-config-dir when HOME is unavailable",
            )
        })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Read one line from stdin without echo when it is a terminal.
fn read_hidden_line(prompt: &str, code: &'static str) -> Result<String> {
    use std::{
        io::{BufRead, IsTerminal},
        os::fd::{AsFd, AsRawFd},
    };
    let io = |e: std::io::Error| Error::new(code, e.to_string());
    let stdin = std::io::stdin();
    let saved = if stdin.is_terminal() {
        eprintln!("{prompt}");
        let saved = herdr_relay::terminal::TerminalState::capture(stdin.as_fd()).map_err(io)?;
        let mut mode = std::mem::MaybeUninit::uninit();
        if unsafe { libc::tcgetattr(stdin.as_raw_fd(), mode.as_mut_ptr()) } != 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        let mut mode = unsafe { mode.assume_init() };
        mode.c_lflag &= !libc::ECHO;
        if unsafe { libc::tcsetattr(stdin.as_raw_fd(), libc::TCSANOW, &mode) } != 0 {
            return Err(io(std::io::Error::last_os_error()));
        }
        Some(saved)
    } else {
        None
    };
    let mut line = String::new();
    let read = stdin.lock().read_line(&mut line);
    if saved.is_some() {
        drop(saved);
        eprintln!();
    }
    read.map_err(io)?;
    Ok(line.trim().to_owned())
}

async fn read_invitation() -> Result<String> {
    use std::{
        io::{BufRead, IsTerminal, Read},
        os::fd::{AsFd, AsRawFd},
    };
    let stdin = std::io::stdin();
    let mut interrupt = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
        .map_err(input_io)?;
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(input_io)?;
    let mut hangup =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup()).map_err(input_io)?;
    let terminal = if stdin.is_terminal() {
        eprintln!("Paste the pairing invitation, then press Enter (input hidden):");
        let saved =
            herdr_relay::terminal::TerminalState::capture(stdin.as_fd()).map_err(input_io)?;
        let mut mode = std::mem::MaybeUninit::uninit();
        if unsafe { libc::tcgetattr(stdin.as_raw_fd(), mode.as_mut_ptr()) } != 0 {
            return Err(input_io(std::io::Error::last_os_error()));
        }
        let mut mode = unsafe { mode.assume_init() };
        // Avoid canonical line-length truncation for long pasted invitations.
        mode.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON);
        mode.c_cc[libc::VMIN] = 1;
        mode.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(stdin.as_raw_fd(), libc::TCSANOW, &mode) } != 0 {
            return Err(input_io(std::io::Error::last_os_error()));
        }
        Some(saved)
    } else {
        None
    };
    let reader = tokio::task::spawn_blocking(|| {
        let mut bytes = Vec::new();
        std::io::stdin()
            .lock()
            .take(8193)
            .read_until(b'\n', &mut bytes)
            .map_err(input_io)?;
        if bytes.len() > 8192 {
            return Err(Error::new(
                "PAIR_INVALID",
                "pairing invitation exceeds size limit",
            ));
        }
        String::from_utf8(bytes)
            .map(|s| s.trim().to_owned())
            .map_err(|_| Error::new("PAIR_INVALID", "invalid pairing invitation encoding"))
    });
    let result = tokio::select! {
        result = reader => result.map_err(|_| Error::new("PAIR_INPUT", "unable to read invitation"))?,
        _ = interrupt.recv() => Err(Error::new("CANCELLED", "pairing cancelled")),
        _ = terminate.recv() => Err(Error::new("CANCELLED", "pairing cancelled")),
        _ = hangup.recv() => Err(Error::new("CANCELLED", "pairing cancelled")),
    };
    if let Some(terminal) = terminal {
        drop(terminal);
        eprintln!();
    }
    result
}
fn input_io(error: std::io::Error) -> Error {
    Error::new("PAIR_INPUT", error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_hosted_settings_can_come_from_the_environment() {
        let _environment = super::cli_tests::ENVIRONMENT
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("HERDR_RELAY_ORIGIN", "https://relay.example.org");
            std::env::set_var("HERDR_RELAY_PROFILE", "/tmp/relay-profile");
            std::env::set_var("HERDR_RELAY_HERDR", "/opt/herdr/bin/herdr");
        }
        let cli = Cli::try_parse_from(["herdr-relay", "targets", "--host", "x"]).unwrap();
        assert_eq!(
            cli.profile.as_deref(),
            Some(std::path::Path::new("/tmp/relay-profile"))
        );
        assert!(
            matches!(cli.command, Some(Action::Targets { ref origin, .. }) if origin == "https://relay.example.org")
        );
        let cli = Cli::try_parse_from(["herdr-relay", "connect"]).unwrap();
        assert!(
            matches!(cli.command, Some(Action::Connect { ref origin, ref herdr, .. })
            if origin == "https://relay.example.org" && herdr == std::path::Path::new("/opt/herdr/bin/herdr"))
        );
        assert!(
            matches!(default_action(&herdr_relay::config::Config::default()), Action::Connect { ref origin, ref herdr, .. }
            if origin == "https://relay.example.org" && herdr == std::path::Path::new("/opt/herdr/bin/herdr"))
        );
        // An explicit flag still wins over the environment.
        let cli =
            Cli::try_parse_from(["herdr-relay", "serve", "--origin", "https://other.example"])
                .unwrap();
        assert!(
            matches!(cli.command, Some(Action::Serve { ref origin, .. }) if origin == "https://other.example")
        );
        unsafe {
            std::env::remove_var("HERDR_RELAY_ORIGIN");
            std::env::remove_var("HERDR_RELAY_PROFILE");
            std::env::remove_var("HERDR_RELAY_HERDR");
        }
        let cli = Cli::try_parse_from(["herdr-relay", "serve"]).unwrap();
        assert!(
            matches!(cli.command, Some(Action::Serve { ref origin, .. }) if origin == DEFAULT_RELAY_ORIGIN)
        );
    }
    #[test]
    fn config_file_values_sit_below_flags_and_environment() {
        let _environment = super::cli_tests::ENVIRONMENT
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut config = herdr_relay::config::Config::default();
        config.set("origin", "https://file.example", None).unwrap();
        config.set("herdr", "/file/herdr", None).unwrap();
        config.set("profile", "/file/profile", None).unwrap();
        config.set("update-repository", "file/fork", None).unwrap();
        config
            .set("update-url", "https://file-mirror.example", None)
            .unwrap();
        let cli = try_parse_with(["herdr-relay", "targets", "--host", "x"], &config).unwrap();
        assert_eq!(
            cli.profile.as_deref(),
            Some(std::path::Path::new("/file/profile"))
        );
        assert!(
            matches!(cli.command, Some(Action::Targets { ref origin, .. }) if origin == "https://file.example")
        );
        let cli = try_parse_with(["herdr-relay", "connect"], &config).unwrap();
        assert!(
            matches!(cli.command, Some(Action::Connect { ref herdr, .. }) if herdr == std::path::Path::new("/file/herdr"))
        );
        let cli = try_parse_with(["herdr-relay", "update"], &config).unwrap();
        assert!(
            matches!(cli.command, Some(Action::Update { ref repository, ref download_url, .. })
                if repository.as_deref() == Some("file/fork")
                    && download_url.as_deref() == Some("https://file-mirror.example"))
        );
        assert!(
            matches!(default_action(&config), Action::Connect { ref origin, ref herdr, .. }
            if origin == "https://file.example" && herdr == std::path::Path::new("/file/herdr"))
        );
        // The local diagnostic keeps its own placeholder origin.
        let cli = try_parse_with(
            [
                "herdr-relay",
                "identity",
                "diagnose",
                "--peer-profile",
                "/p",
            ],
            &config,
        )
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Action::Identity { action: IdentityAction::Diagnose { ref origin, .. } })
            if origin == "https://diagnostic.invalid"
        ));
        // Environment and flags still win.
        unsafe { std::env::set_var("HERDR_RELAY_ORIGIN", "https://env.example") };
        let cli = try_parse_with(["herdr-relay", "serve"], &config).unwrap();
        assert!(
            matches!(cli.command, Some(Action::Serve { ref origin, .. }) if origin == "https://env.example")
        );
        assert!(
            matches!(default_action(&config), Action::Connect { ref origin, .. } if origin == "https://env.example")
        );
        unsafe { std::env::remove_var("HERDR_RELAY_ORIGIN") };
        let cli = try_parse_with(
            ["herdr-relay", "serve", "--origin", "https://flag.example"],
            &config,
        )
        .unwrap();
        assert!(
            matches!(cli.command, Some(Action::Serve { ref origin, .. }) if origin == "https://flag.example")
        );
    }

    #[test]
    fn host_commands_parse() {
        let empty = herdr_relay::config::Config::default();
        assert!(matches!(
            try_parse_with(["herdr-relay", "hosts", "list"], &empty)
                .unwrap()
                .command,
            Some(Action::Hosts {
                action: HostsAction::List { .. }
            })
        ));
        assert!(matches!(
            try_parse_with(["herdr-relay", "hosts", "remove", "ubuntu"], &empty).unwrap().command,
            Some(Action::Hosts { action: HostsAction::Remove { ref host, .. } }) if host == "ubuntu"
        ));
    }

    #[test]
    fn config_commands_parse() {
        let empty = herdr_relay::config::Config::default();
        assert!(matches!(
            try_parse_with(["herdr-relay", "config", "show"], &empty)
                .unwrap()
                .command,
            Some(Action::Config {
                action: ConfigAction::Show
            })
        ));
        assert!(matches!(
            try_parse_with(["herdr-relay", "config", "set", "origin", "https://x.example"], &empty)
                .unwrap()
                .command,
            Some(Action::Config { action: ConfigAction::Set { ref key, value: Some(ref value), origin: None } })
            if key == "origin" && value == "https://x.example"
        ));
        assert!(matches!(
            try_parse_with(
                [
                    "herdr-relay",
                    "config",
                    "set",
                    "token",
                    "--origin",
                    "https://x.example"
                ],
                &empty
            )
            .unwrap()
            .command,
            Some(Action::Config {
                action: ConfigAction::Set {
                    value: None,
                    origin: Some(_),
                    ..
                }
            })
        ));
        assert!(matches!(
            try_parse_with(
                [
                    "herdr-relay",
                    "config",
                    "unset",
                    "token",
                    "--origin",
                    "https://x.example"
                ],
                &empty
            )
            .unwrap()
            .command,
            Some(Action::Config {
                action: ConfigAction::Unset { .. }
            })
        ));
    }
}
