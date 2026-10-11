use crate::{
    Error, Result,
    identity::Identity,
    names::{self, PairLink},
    targets::{Definition, Locator, Store},
    tui::{Choice, Screen},
    user_service,
};
use crossterm::event::{Event, KeyCode};
use futures_util::{StreamExt, stream};
use std::{path::Path, time::Duration};

pub const DEFAULT_ORIGIN: &str = "https://herdr-relay.hackerchai.com";

/// A host row's status with how the host was reached.
fn host_status(status: &str, route: Option<crate::direct::Route>) -> String {
    match route {
        Some(route) => format!("{status} · {route}"),
        None => status.to_owned(),
    }
}

fn needs_token(error: &Error) -> bool {
    matches!(error.code, "RELAY_AUTH_REQUIRED" | "RELAY_AUTH_INVALID")
}

/// Ask for a self-hosted relay's access token and save it in config.toml.
async fn ask_token(screen: &mut Screen, origin: &str) -> Result<bool> {
    loop {
        let Some(token) = screen
            .input(
                "Relay access token",
                &format!(
                    "{origin} requires an access token from its operator. It is saved in config.toml and sent only to this relay."
                ),
                true,
            )
            .await?
        else {
            return Ok(false);
        };
        match crate::config::path()
            .and_then(|path| crate::config::save_token(&path, origin, &token))
        {
            Ok(()) => return Ok(true),
            Err(e) => {
                screen
                    .message("Could not save the token", &e.to_string())
                    .await?
            }
        }
    }
}

#[derive(Clone)]
struct Session {
    name: String,
    locator: Locator,
    selected: bool,
    online: bool,
}
fn discover(config: &Path, existing: &[Definition]) -> Vec<Session> {
    let mut sessions: Vec<_> = existing
        .iter()
        .map(|t| Session {
            name: t.name.clone(),
            locator: t.locator.clone(),
            selected: t.enabled,
            online: t.locator.resolve().is_ok(),
        })
        .collect();
    let mut names = vec!["default".to_owned()];
    if let Ok(entries) = std::fs::read_dir(config.join("sessions")) {
        names.extend(entries.filter_map(|e| e.ok()?.file_name().into_string().ok()));
    }
    names.sort();
    names.dedup();
    for name in names {
        let locator = Locator::Session {
            config_dir: config.to_path_buf(),
            session: name.clone(),
        };
        if let Ok(native) = locator.resolve() {
            if sessions.iter().any(|s| {
                s.locator == locator
                    || s.locator
                        .resolve()
                        .is_ok_and(|other| other.same_instance(&native))
            }) {
                continue;
            }
            sessions.push(Session {
                name,
                locator,
                selected: false,
                online: true,
            });
        }
    }
    sessions
}

pub async fn serve(profile: &Path, origin: &str, config: &Path) -> Result<()> {
    let saved = crate::pairing::Store::saved_origin(profile)?;
    let origin = if origin == DEFAULT_ORIGIN {
        saved.as_deref().unwrap_or(origin)
    } else {
        origin
    };
    let mut screen = Screen::new()?;
    let mut autostart = user_service::configuration(profile)?
        .map(|c| c.start_at_login)
        .unwrap_or(true);
    loop {
        let expected = if profile.join("identity").exists() {
            Store::open(profile)?.list()?
        } else {
            vec![]
        };
        let mut sessions = discover(config, &expected);
        let mut selected: usize = 0;
        let apply = loop {
            let rows: Vec<_> = sessions
                .iter()
                .map(|s| {
                    format!(
                        "[{}] {}  ({})",
                        if s.selected { 'x' } else { ' ' },
                        names::display(&s.name),
                        if s.online {
                            "running"
                        } else {
                            "offline, saved"
                        }
                    )
                })
                .collect();
            let hint = if rows.is_empty() {
                "No running Herdr sessions. Run herdr in another terminal, detach, then press r.\nRelay only shares sessions you explicitly select."
            } else {
                "Space selects; Enter applies. Unchecked sessions stop sharing."
            };
            screen.render(
                &format!("Share sessions on {}", names::hostname()),
                &rows,
                selected,
                &format!(
                    "{hint}\nStart at login: {} (a toggles)\nSleep protection: automatic while serving (? for lid-close limits)",
                    if autostart { "on" } else { "off" }
                ),
                "Space Select    Enter Apply    a Autostart    r Refresh    ? Help    Esc Exit",
            )?;
            if let Event::Key(key) = screen.event().await? {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => return Ok(()),
                    KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => selected = (selected + 1).min(sessions.len().saturating_sub(1)),
                    KeyCode::Char(' ') if !sessions.is_empty() => sessions[selected].selected = !sessions[selected].selected,
                    KeyCode::Char('a') => autostart = !autostart,
                    KeyCode::Char('r') => break false,
                    KeyCode::Enter if !sessions.is_empty() => break true,
                    KeyCode::Char('?') | KeyCode::F(1) => screen.help("Only selected sessions are shared; Herdr processes are never started or stopped.\nAutostart runs at login. Linux boot mode: service autostart boot.\nSleep protection starts with the server; MacBook lid close remains OS controlled.\nAfter setup choose Sleep protection status, or run herdr-relay serve --help.").await?,
                    _ => {},
                }
            }
        };
        if !apply {
            continue;
        }
        // A self-hosted relay may require a token; ask now instead of letting
        // the background service fail out of sight.
        if origin != crate::config::PUBLIC_ORIGIN {
            Identity::initialize(profile)?;
            let relay = crate::relay::Relay::new(profile, origin, None)?;
            match screen
                .wait("Checking the relay", relay.check_access())
                .await
            {
                Err(e) if needs_token(&e) => {
                    if !ask_token(&mut screen, origin).await? {
                        continue;
                    }
                    // A running service rereads config.toml when it starts again.
                    if user_service::configuration(profile)?.is_some() {
                        user_service::stop(profile).await?;
                    }
                }
                Err(e) if e.code == "CANCELLED" => return Err(e),
                _ => {}
            }
        }
        let chosen: Vec<_> = sessions
            .iter()
            .filter(|s| s.selected)
            .map(|s| (s.name.clone(), s.locator.clone()))
            .collect();
        let result = async {
            Identity::initialize(profile)?;
            crate::pairing::Store::open(profile, origin)?;
            // Validate the installed origin before committing the selection.
            if user_service::configuration(profile)?.is_some_and(|c| c.origin != origin) {
                return Err(Error::new(
                    "ORIGIN_MISMATCH",
                    "the installed service uses another Relay; switch it explicitly first",
                ));
            }
            Store::open(profile)?.apply_selection(&expected, &chosen)?;
            user_service::ensure(profile, origin, autostart).await?;
            crate::service::reload_targets(profile).await?;
            for _ in 0..40 {
                if crate::service::status(profile)
                    .await
                    .is_ok_and(|(_, online, _)| online)
                {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(Error::new(
                "RELAY_CONNECTING",
                "sharing is saved and the service is running, but the Relay is not online yet; check service status and retry",
            ))
        };
        match screen
            .wait("Applying sharing and starting service", result)
            .await
        {
            Ok(()) => {
                let link = invitation_screen(&mut screen, profile).await?;
                drop(screen);
                if let Some(link) = link {
                    println!(
                        "Pairing link (one use, expires in 10 minutes):\n{link}\n\nSharing continues in the background."
                    );
                }
                return Ok(());
            }
            Err(e) if e.code == "BACK" => return Ok(()),
            Err(e) if e.code == "CANCELLED" => return Err(e),
            Err(e) => {
                screen
                    .message("Setup needs attention", &e.to_string())
                    .await?
            }
        }
    }
}

async fn invitation_screen(screen: &mut Screen, profile: &Path) -> Result<Option<String>> {
    loop {
        let invitation = screen
            .wait(
                "Creating pairing link",
                crate::service::create_invitation(profile),
            )
            .await?;
        let link = PairLink::encode(invitation, names::hostname())?;
        let rows = vec![
            "Copy pairing link".into(),
            "Print link and finish".into(),
            "Generate a fresh link".into(),
            "Sleep protection status".into(),
            "Done".into(),
        ];
        loop {
            match screen.choose("Sharing is running", &rows, "On the other computer: herdr-relay pair, then paste the link.\nOne-use link, valid for 10 minutes, for all shared sessions.\nSleep protection status shows inhibitors and lid-close limits.", false).await? {
                Choice::Pick(0) => match copy_link(&link).await {
                    Ok(()) => screen.message("Copied", "Paste into herdr-relay pair on the client. The link expires in 10 minutes.").await?,
                    Err(_) => screen.message("Clipboard unavailable", "Choose Print link and finish, then copy the link from your terminal.").await?,
                },
                Choice::Pick(1) => return Ok(Some(link)),
                Choice::Pick(2) | Choice::Refresh => break,
                Choice::Pick(3) => {
                    let power = crate::service::power_summary(profile).await;
                    screen.message("Sleep protection", &format!("{power}\n\nMacBook closed-display use needs power, an external display and input devices.\nA VM cannot keep its physical host awake.\nStop the service to release locks. Details: herdr-relay serve --help.")).await?;
                },
                Choice::Pick(_) | Choice::Back => return Ok(None),
                Choice::Rename(_) => {},
            }
        }
    }
}
async fn copy_link(value: &str) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let programs: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("/usr/bin/pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };
    for (program, args) in programs {
        let result = tokio::time::timeout(Duration::from_secs(2), async {
            let mut child = tokio::process::Command::new(program)
                .args(*args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(value.as_bytes()).await?;
            }
            child.wait().await
        })
        .await;
        if matches!(result, Ok(Ok(status)) if status.success()) {
            return Ok(());
        }
    }
    Err(Error::new("CLIPBOARD", "clipboard unavailable"))
}

/// `local` is the loopback development transport, as with `--relay-local-url`.
async fn pair_in(
    screen: &mut Screen,
    profile: &Path,
    local: Option<&str>,
) -> Result<Option<(String, String)>> {
    let mut retry: Option<String> = None;
    loop {
        let value = match retry.take() {
            Some(value) => value,
            None => {
                let Some(value) = screen
                    .input(
                        "Pair a host",
                        "Paste the complete pairing link from herdr-relay serve.",
                        true,
                    )
                    .await?
                else {
                    return Ok(None);
                };
                value
            }
        };
        let result = async {
            let link = PairLink::decode(&value)?;
            Identity::initialize(profile)?;
            let relay = crate::relay::Relay::new(profile, &link.invitation.origin, local)?;
            relay.accept(&link.invitation).await?;
            let host = names::remember(
                profile,
                &link.invitation.origin,
                &link.invitation.publisher,
                link.hostname.as_deref(),
                None,
            )?;
            Ok((host.origin, host.identity, host.alias))
        };
        match screen.wait("Pairing with the host", result).await {
            Ok((origin, identity, alias)) => {
                screen.message("Paired", &format!("You can now access {alias}.\nChoose a shared session next. Reverse access is not granted.")).await?;
                return Ok(Some((origin, identity)));
            }
            Err(e) if e.code == "BACK" => continue,
            Err(e) if e.code == "CANCELLED" => return Err(e),
            Err(e) if needs_token(&e) => {
                // Retry the same link once the relay's token is saved.
                let origin = PairLink::decode(&value)?.invitation.origin;
                if ask_token(screen, &origin).await? {
                    retry = Some(value);
                }
            }
            Err(e) => screen.message("Could not pair", &e.to_string()).await?,
        }
    }
}

pub async fn pair(profile: &Path, herdr: &Path) -> Result<()> {
    let mut screen = Screen::new()?;
    let paired = pair_in(&mut screen, profile, None).await?;
    drop(screen);
    if let Some((origin, identity)) = paired {
        connect(profile, &origin, herdr, Some(identity), None).await?;
    }
    Ok(())
}

pub async fn connect(
    profile: &Path,
    origin: &str,
    herdr: &Path,
    mut selected_host: Option<String>,
    local: Option<&str>,
) -> Result<()> {
    let mut screen = Screen::new()?;
    let mut origin = if origin == DEFAULT_ORIGIN {
        crate::pairing::Store::saved_origin(profile)?.unwrap_or_else(|| origin.to_owned())
    } else {
        origin.to_owned()
    };
    loop {
        if !profile.join("identity").exists() {
            let rows = vec!["Pair a host using a link".into()];
            match screen
                .choose(
                    "Welcome",
                    &rows,
                    "Run herdr-relay serve on the computer you want to access.",
                    false,
                )
                .await?
            {
                Choice::Back => return Ok(()),
                Choice::Pick(_) => {
                    if let Some((new_origin, host)) = pair_in(&mut screen, profile, local).await? {
                        origin = new_origin;
                        selected_host = Some(host);
                    }
                }
                _ => {}
            }
            if !profile.join("identity").exists() {
                continue;
            }
        }
        let grants = crate::pairing::Store::open(profile, &origin)?
            .grants()?
            .accessible_hosts;
        let hosts: Vec<_> = grants
            .iter()
            .map(|g| names::remember(profile, &origin, &g.device_id, None, None))
            .collect::<Result<_>>()?;
        let relay = crate::relay::Relay::new(profile, &origin, local)?;
        let host = if let Some(identity) = selected_host.take() {
            hosts.iter().find(|h| h.identity == identity).cloned()
        } else {
            let mut rows: Vec<_> = hosts
                .iter()
                .map(|h| format!("{}   checking…", h.alias))
                .collect();
            rows.push("+ Pair another host".into());
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
            let jobs: Vec<_> = hosts
                .iter()
                .enumerate()
                .map(|(index, host)| (index, host.clone(), relay.clone()))
                .collect();
            let mut tasks = tokio::task::JoinSet::new();
            tasks.spawn(async move {
                let mut checks = stream::iter(jobs)
                    .map(|(index, host, relay)| async move {
                        let status = match tokio::time::timeout(
                            Duration::from_secs(6),
                            relay.targets(&host.identity),
                        )
                        .await
                        {
                            Ok(Ok(_)) => host_status("online", relay.route(&host.identity)),
                            Ok(Err(e)) if e.code == "NOT_AUTHORIZED" => "access revoked".into(),
                            Ok(Err(e)) if needs_token(&e) => "relay token required".into(),
                            _ => "offline / unavailable".into(),
                        };
                        // A refreshed hostname takes effect on the next menu load.
                        (index, format!("{}   {status}", host.alias))
                    })
                    .buffer_unordered(4);
                while let Some(update) = checks.next().await {
                    if tx.send(update).is_err() {
                        break;
                    }
                }
            });
            let choice = screen
                .choose_live(
                    "Choose a host",
                    &rows,
                    "Select a host to browse its shared Herdr sessions. Aliases are saved locally.",
                    true,
                    Some(&mut rx),
                )
                .await?;
            drop(tasks);
            match choice {
                Choice::Back => return Ok(()),
                Choice::Refresh => continue,
                Choice::Rename(i) if i < hosts.len() => {
                    if let Some(alias) = screen.input("Rename host", "Choose a unique local alias. This does not change the host's identity or authorization.", false).await?
                        && let Err(e) = names::remember(profile, &origin, &hosts[i].identity, None, Some(&alias)) { screen.message("Could not rename", &e.to_string()).await?; }
                    continue;
                }
                Choice::Pick(i) if i < hosts.len() => Some(hosts[i].clone()),
                Choice::Pick(_) => {
                    if let Some((new_origin, host)) = pair_in(&mut screen, profile, local).await? {
                        origin = new_origin;
                        selected_host = Some(host);
                    }
                    continue;
                }
                _ => continue,
            }
        };
        let Some(host) = host else {
            continue;
        };
        loop {
            let targets = match screen
                .wait("Loading shared sessions", relay.targets(&host.identity))
                .await
            {
                Ok(targets) => targets,
                Err(e) if e.code == "BACK" => break,
                Err(e) if e.code == "CANCELLED" => return Err(e),
                Err(e) if needs_token(&e) => {
                    // The next pass builds a relay client with the saved token.
                    if ask_token(&mut screen, &origin).await? {
                        selected_host = Some(host.identity.clone());
                    }
                    break;
                }
                Err(e) => {
                    screen
                        .message(
                            "Host unavailable",
                            &format!(
                                "{}: {}\nRefresh later or check the server's service status.",
                                host.alias, e.code
                            ),
                        )
                        .await?;
                    break;
                }
            };
            let rows: Vec<_> = targets
                .iter()
                .map(|t| {
                    format!(
                        "{}   {}",
                        names::display(&t.name),
                        names::display(&t.status)
                    )
                })
                .collect();
            let connection = relay
                .route(&host.identity)
                .map_or_else(|| "unknown".into(), |route| route.to_string());
            let hint = format!(
                "{}\nConnection: {connection}",
                if rows.is_empty() {
                    "No sessions are currently shared. Select sessions with herdr-relay serve on this host."
                } else {
                    "Enter connects. Detach from Herdr with Ctrl-B, release, then Q to return here."
                }
            );
            match screen
                .choose(&format!("Sessions on {}", host.alias), &rows, &hint, false)
                .await?
            {
                Choice::Back => break,
                Choice::Refresh => continue,
                Choice::Pick(index) => {
                    let target = &targets[index];
                    if target.status != "online" {
                        screen.message("Session unavailable", "This session is offline or busy. Refresh after it becomes available.").await?;
                        continue;
                    }
                    if let Err(e) = screen
                        .wait("Checking Herdr client", crate::client::check_version(herdr))
                        .await
                    {
                        if e.code == "BACK" {
                            continue;
                        }
                        if e.code == "CANCELLED" {
                            return Err(e);
                        }
                        screen.message("Herdr is required", &e.to_string()).await?;
                        continue;
                    }
                    // Stop rendering and release all input ownership before native attach.
                    drop(screen);
                    let result = crate::client::connect_remote(
                        relay.clone(),
                        &host.identity,
                        &target.id,
                        herdr,
                        "herdr-relay",
                    )
                    .await;
                    screen = Screen::new()?;
                    if let Err(e) = result {
                        if e.code == "CANCELLED" {
                            return Ok(());
                        }
                        let via = relay
                            .route(&host.identity)
                            .map_or_else(String::new, |route| format!(" (via {route})"));
                        screen
                            .message(
                                "Connection ended",
                                &format!(
                                    "{} / {}{via}: {}",
                                    host.alias,
                                    names::display(&target.name),
                                    e.message
                                ),
                            )
                            .await?;
                    }
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_rows_say_whether_the_host_is_reached_directly_or_through_the_relay() {
        use crate::direct::Route;
        let lan = Route::Direct("192.168.1.20:47120".parse().unwrap());
        let tailnet = Route::Direct("100.101.1.2:47120".parse().unwrap());
        assert_eq!(
            host_status("online", Some(lan)),
            "online · direct (LAN 192.168.1.20)"
        );
        assert_eq!(
            host_status("online", Some(tailnet)),
            "online · direct (tailnet 100.101.1.2)"
        );
        assert_eq!(host_status("online", Some(Route::Relay)), "online · relay");
        assert_eq!(
            host_status("offline / unavailable", None),
            "offline / unavailable"
        );
    }
    #[test]
    fn discovery_keeps_saved_offline_sessions_and_enabled_state() {
        let tmp = tempfile::tempdir().unwrap();
        let locator = Locator::Session {
            config_dir: tmp.path().into(),
            session: "work".into(),
        };
        let existing = vec![Definition {
            id: "a".repeat(32),
            name: "work".into(),
            locator,
            enabled: true,
        }];
        let sessions = discover(tmp.path(), &existing);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].selected);
        assert!(!sessions[0].online);
    }
}
