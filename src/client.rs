use crate::{
    Error, Result,
    native::NativeTarget,
    transport::{NativeEnd, bridge_native},
};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::{
    io::{IsTerminal, Write},
    path::Path,
    process::{ExitStatus, Stdio},
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    net::UnixListener,
    process::{Child, Command},
    signal::unix::{SignalKind, signal},
    time::timeout,
};

fn command(binary: &Path) -> Command {
    let mut command = Command::new(binary);
    // Herdr session/socket overrides and nesting markers must not select a
    // different local instance. Preserve the user's terminal and XDG settings.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("HERDR_") {
            command.env_remove(key);
        }
    }
    command.kill_on_drop(true);
    command
}

pub async fn check_version(binary: &Path) -> Result<()> {
    let mut child = command(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| Error::new("CLIENT_START_FAILED", e.to_string()))?;
    let mut output = Vec::new();
    let mut stdout = child.stdout.take().unwrap().take(128);
    let result = timeout(Duration::from_secs(3), async {
        stdout.read_to_end(&mut output).await?;
        child.wait().await
    })
    .await;
    match result {
        Ok(Ok(status))
            if status.success()
                && (output == b"herdr 0.9.3\n"
                    || output == b"herdr 0.9.3-custom.relay-control-bound-1\n") =>
        {
            Ok(())
        }
        Ok(Ok(_)) => Err(Error::new(
            "INCOMPATIBLE_HERDR",
            "use Herdr 0.9.3 or the relay-control-bound-1 build",
        )),
        Ok(Err(e)) => Err(Error::new("CLIENT_START_FAILED", e.to_string())),
        Err(_) => {
            let _ = child.kill().await;
            Err(Error::new(
                "CLIENT_START_TIMEOUT",
                "Herdr version check exceeded 3 seconds",
            ))
        }
    }
}

/// Run one foreground native client. This function never launches a server.
pub async fn run_local(target: NativeTarget, binary: &Path, reattach: &str) -> Result<()> {
    check_version(binary).await?;
    let _reservation = target.reserve()?;
    run(binary, reattach, || target.connect()).await
}
pub async fn run_remote(
    prepared: crate::relay::PreparedAttach,
    binary: &Path,
    reattach: &str,
) -> Result<()> {
    check_version(binary).await?;
    run(binary, reattach, || prepared.start()).await
}

/// Own the terminal across fresh attach attempts. There is never a native
/// client or buffered native input in the offline/establishing state.
pub async fn connect_remote(
    relay: crate::relay::Relay,
    publisher: &str,
    target: &str,
    binary: &Path,
    reattach: &str,
) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Error::new(
            "TERMINAL_REQUIRED",
            "run connect from an interactive terminal",
        ));
    }
    let mut interrupt = signal(SignalKind::interrupt()).map_err(local_io)?;
    let mut terminate = signal(SignalKind::terminate()).map_err(local_io)?;
    let mut hangup = signal(SignalKind::hangup()).map_err(local_io)?;
    let mut backoff = crate::retry::Backoff::default();
    let mut previous: Option<String> = None;
    let mut delay = Duration::ZERO;
    let mut native_failure: Option<(Error, String)> = None;
    loop {
        let input =
            crate::terminal::OfflineInput::capture(std::io::stdin().as_fd()).map_err(local_io)?;
        let mut drain = tokio::time::interval(Duration::from_millis(20));
        let prepared = {
            let attempt = async {
                crate::retry::wait(delay).await;
                relay.check_identity()?;
                check_version(binary).await?;
                if let Some(channel) = previous.as_ref() {
                    relay.release_channel(publisher, channel).await?;
                    previous = None;
                }
                if native_failure.is_some() {
                    let targets = relay.targets(publisher).await?;
                    let current = targets.iter().find(|t| t.id == target).ok_or_else(|| {
                        Error::new("TARGET_NOT_SHARED", "target is no longer shared")
                    })?;
                    if current.status != "offline"
                        && current.runtime_id == native_failure.as_ref().unwrap().1
                    {
                        if current.status == "busy" {
                            return Err(Error::new(
                                "TARGET_BUSY",
                                "another client occupies the target",
                            ));
                        }
                        return Err(native_failure.take().unwrap().0);
                    }
                    native_failure = None;
                }
                let channel = crate::control::random_id()?;
                previous = Some(channel.clone());
                relay.prepare_attach(publisher, target, &channel).await
            };
            tokio::pin!(attempt);
            loop {
                tokio::select! {
                    biased;
                    _ = interrupt.recv() => break Err(Error::new("CANCELLED","connection cancelled")),
                    _ = terminate.recv() => break Err(Error::new("CANCELLED","connection cancelled")),
                    _ = hangup.recv() => break Err(Error::new("CANCELLED","terminal closed")),
                    _ = drain.tick() => if input.discard().map_err(local_io)? { break Err(Error::new("CANCELLED","connection cancelled")); },
                    result = &mut attempt => break result,
                }
            }
        };
        // TCSAFLUSH drops even the last bytes arriving between drain ticks.
        drop(input);
        let began = tokio::time::Instant::now();
        let runtime = prepared.as_ref().ok().map(|p| p.runtime_id().to_owned());
        // Only relay channels have relay state to release before a retry.
        if prepared
            .as_ref()
            .is_ok_and(|p| p.route() != crate::direct::Route::Relay)
        {
            previous = None;
        }
        let result = match prepared {
            Ok(prepared) => run(binary, reattach, || prepared.start()).await,
            Err(error) => Err(error),
        };
        if began.elapsed() >= Duration::from_secs(60) {
            backoff.reset();
        }
        match result {
            Ok(()) => return Ok(()), // Explicit native detach never retries.
            Err(error) => {
                // Herdr can send its own server-shutdown frame before the
                // transport EOF. Its nonzero client exit alone cannot tell
                // this apart from a broken client. Recheck the authenticated
                // target generation once; only offline/replaced runtimes retry.
                if error.code == "NATIVE_CLIENT_FAILED"
                    && let Some(runtime) = runtime
                {
                    native_failure = Some((error, runtime));
                    delay = Duration::from_millis(500);
                    eprintln!(
                        "Disconnected. Checking target after native client exit; Ctrl-C cancels. Offline input is discarded."
                    );
                    continue;
                }
                let Some(next) = backoff.next(&error, crate::retry::jitter()?) else {
                    return Err(error);
                };
                delay = next;
                eprintln!(
                    "Disconnected ({}). Retrying in {:.1}s; Ctrl-C cancels. Offline input is discarded.",
                    error.code,
                    delay.as_secs_f64()
                );
            }
        }
    }
}
async fn run<F, Fut, S>(binary: &Path, reattach: &str, connect: F) -> Result<()>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<S>>,
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut interrupt = signal(SignalKind::interrupt()).map_err(local_io)?;
    let mut terminate = signal(SignalKind::terminate()).map_err(local_io)?;
    let mut hangup = signal(SignalKind::hangup()).map_err(local_io)?;
    let cancel = async {
        tokio::select! {
            _ = interrupt.recv() => {}, _ = terminate.recv() => {}, _ = hangup.recv() => {},
        }
    };
    tokio::pin!(cancel);

    let mut terminal = Terminal::capture()?;
    // /tmp keeps the AF_UNIX pathname below the macOS 104-byte limit.
    let directory = tempfile::Builder::new()
        .prefix("hr-")
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir_in("/tmp")
        .map_err(local_io)?;
    let path = directory.path().join("client.sock");
    let listener = UnixListener::bind(&path).map_err(local_io)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(local_io)?;
    let mut child = command(binary)
        .arg("client")
        .env("HERDR_CLIENT_SOCKET_PATH", &path)
        .env("HERDR_REMOTE_KEYBINDINGS", "local")
        .env("HERDR_REATTACH_COMMAND", reattach)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| Error::new("CLIENT_START_FAILED", e.to_string()))?;
    terminal.emergency_cleanup = true;
    let result = async {
        let native = tokio::select! {
            result = timeout(Duration::from_secs(10), listener.accept()) => {
                result.map_err(|_| Error::new("CLIENT_START_TIMEOUT", "native client did not connect within 10 seconds"))?
                    .map_err(local_io)?.0
            },
            status = child.wait() => return client_status(status.map_err(local_io)?),
            _ = &mut cancel => return Err(Error::new("CANCELLED", "connection cancelled")),
        };
        if native.peer_cred().map_err(local_io)?.uid() != unsafe { libc::geteuid() } {
            return Err(Error::new("UNSAFE_CLIENT", "local client belongs to another user"));
        }
        // Open the target only after the client is ready to send native hello.
        let server = tokio::select! {
            result = timeout(Duration::from_secs(4), connect()) => result
                .map_err(|_| Error::new("HANDSHAKE_TIMEOUT", "native socket connection timed out"))??,
            _ = &mut cancel => return Err(Error::new("CANCELLED", "connection cancelled")),
        };
        let transfer = bridge_native(native, server, Duration::from_secs(4));
        tokio::pin!(transfer);
        tokio::select! {
            biased;
            status = child.wait() => client_status(status.map_err(local_io)?),
            _ = &mut cancel => Err(Error::new("CANCELLED", "connection cancelled")),
            result = &mut transfer => {
                // Dropping transfer below closes both owned streams before
                // waiting for the native process to restore its terminal.
                match result? {
                    NativeEnd::Client => Ok(()),
                    NativeEnd::Server => Err(Error::new("CONNECTION_LOST","remote native stream ended")),
                }
            },
        }
    }.await;
    // The transfer future and both sockets have been dropped here. No queued
    // native input survives cleanup or a later attach.
    let status = finish_client(&mut child).await;
    let native_detached = status.as_ref().is_ok_and(|s| s.success());
    if native_detached {
        terminal.emergency_cleanup = false;
    }
    drop(terminal);
    drop(listener);
    drop(directory);
    // On a fast local transport, the server's detach EOF can win the select
    // before the client's successful exit becomes observable. The native
    // client's clean exit distinguishes detach from server loss (nonzero).
    if !(native_detached && result.as_ref().is_err_and(|e| e.code == "CONNECTION_LOST")) {
        result?;
    }
    client_status(status?)
}

async fn finish_client(child: &mut Child) -> Result<ExitStatus> {
    if let Ok(status) = timeout(Duration::from_millis(500), child.wait()).await {
        return status.map_err(local_io);
    }
    // Only signal the Child handle owned by this connect; never search by name
    // or signal a process group containing a Herdr server or pane process.
    child.start_kill().map_err(local_io)?;
    child.wait().await.map_err(local_io)
}

fn client_status(status: ExitStatus) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        Err(Error::new(
            "NATIVE_CLIENT_FAILED",
            format!("Herdr client exited with {status}; see its diagnostic above"),
        ))
    }
}

fn local_io(e: std::io::Error) -> Error {
    Error::new("LOCAL_IO", e.to_string())
}

struct Terminal {
    state: Option<crate::terminal::TerminalState>,
    emergency_cleanup: bool,
}

impl Terminal {
    fn capture() -> Result<Self> {
        if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
            return Err(Error::new(
                "TERMINAL_REQUIRED",
                "run connect from an interactive terminal",
            ));
        }
        Ok(Self {
            state: Some(
                crate::terminal::TerminalState::capture(std::io::stdin().as_fd())
                    .map_err(local_io)?,
            ),
            emergency_cleanup: false,
        })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        drop(self.state.take());
        if !self.emergency_cleanup {
            return;
        }
        // A successful native client restores its own display/keyboard stack.
        // On abnormal exit also reset modes observed in Herdr 0.9.3 startup.
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(b"\x1b[?2026l\x1b[?7h\x1b[?9l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1005l\x1b[?1006l\x1b[?1015l\x1b[?1016l\x1b[?2004l\x1b[?2031l\x1b[=0u\x1b[0m\x1b[0 q\x1b[?25h\x1b[?1049l");
        let _ = out.flush();
    }
}
