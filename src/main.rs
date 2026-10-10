use clap::{Parser, Subcommand};
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
    /// Update from the latest complete stable GitHub Release.
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
    },
    /// Run the Relay service without a terminal (used by service managers).
    #[command(hide = true)]
    Daemon {
        #[arg(long, env = "HERDR_RELAY_ORIGIN", default_value = DEFAULT_RELAY_ORIGIN)]
        origin: String,
        #[arg(long, hide = true)]
        relay_local_url: Option<String>,
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
    /// Connect an official Herdr 0.9.3 client through a local byte-stream bridge.
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
    if let Err(error) = run(Cli::parse()).await {
        if error.code == "BACK" {
            return;
        }
        eprintln!("{error}");
        std::process::exit(if error.code == "CANCELLED" { 130 } else { 1 });
    }
}

/// Plain `herdr-relay` opens the host menu, honoring the same environment.
fn default_action() -> Action {
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
    Action::Connect {
        origin: var("HERDR_RELAY_ORIGIN").unwrap_or_else(|| DEFAULT_RELAY_ORIGIN.into()),
        host: None,
        target: None,
        herdr: var("HERDR_RELAY_HERDR")
            .unwrap_or_else(|| "herdr".into())
            .into(),
        relay_local_url: None,
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command.unwrap_or_else(default_action) {
        Action::Update {
            check,
            restart_service,
            repository,
        } => {
            let profile = cli.profile.map(Ok).unwrap_or_else(profile_dir)?;
            herdr_relay::update::run(&profile, check, restart_service, repository.as_deref()).await
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
                return herdr_relay::interactive::connect(&profile, &origin, &herdr, None).await;
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
                return herdr_relay::interactive::pair(&profile, std::path::Path::new("herdr"))
                    .await;
            };
            match action {
                PairAction::Create => {
                    let invitation = herdr_relay::service::create_invitation(&profile).await?;
                    eprintln!(
                        "One-time invitation {} expires at {} (Unix time). Grants access to all explicitly shared instances.",
                        invitation.invitation_id, invitation.expires_at
                    );
                    println!("{}", invitation.encode()?);
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
            matches!(default_action(), Action::Connect { ref origin, ref herdr, .. }
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
}
