# macOS candidates and user service — 2026-10-10

Issue: [#13](https://github.com/hackerchai/herdr-relay/issues/13). Native testing
used macOS 27 arm64. The x86_64 binary can run under this machine's Rosetta;
this is not a physical Intel Mac validation claim. That hardware acceptance
remains open. No public release or notarization was performed.

Two local candidate archives were built with Rust 1.99.0 and locked dependencies,
with per-file SHA-256 manifests and adjacent archive checksums. Installers checked
architecture and checksums and ran without Rust or Node. The arm64 candidate is
installed at `~/.local/bin/herdr-relay`; isolated installer destinations were also
verified. Source/build toolchains are needed only by maintainers.

A separate persistent profile under `~/.local/state/herdr-relay-service-check`
held its own device identity. Its launchd label was derived from the canonical
profile path. The definition and config were mode 0600, the installed executable
0700, and the profile 0700. `plutil -lint` passed. A CLI invocation that installed
and started the job exited; the daemon's PPID was 1, and remote operation through
real Cloudflare continued from subsequent invocations. This tests independence
from the launching process, not logging out of the user's GUI session.

The full arm64 manual lifecycle passed:

- Install, start, status, and device pairing through the running service.
- A native client remotely operated the isolated Work session. Repeated start
  kept the same PID; installing while loaded returned `SERVICE_RUNNING`.
- Repeated stop was idempotent. Herdr remained running. Restart reused identity,
  grants and targets, and the existing client reattached and accepted input.
- Reinstalling twice while stopped was safe. The explicit start-at-login option
  generated the corresponding RunAtLoad setting and the resulting job started.
- Native detach returned 0 and restored terminal settings. Repeated uninstall
  was idempotent, left no loaded job, and preserved all identity/grant/target
  file hashes. It did not terminate Herdr.

A startup grant-mirror race was found and fixed during this workflow; see
[reconnect follow-up](2026-10-10-reconnect.md). The controlled live regression
held a publisher connection before sending initial state: access now returned
`409 HOST_OFFLINE`, then correctly returned `403 NOT_AUTHORIZED` after an empty
authoritative state. Managed restart then passed without re-pairing.

Start-at-login is opt-in. No logout/login was forced on the user's machine, and
no before-login operation is claimed. A LaunchAgent runs in the GUI login domain.
Background stderr is discarded; foreground mode provides diagnostics. Fatal
configuration/process exit requires an explicit restart; transient network
failures use the daemon's existing reconnect behavior.

The x86_64 release CLI then installed its own copy as the same isolated user
service and completed pairing. A separate x86_64 CLI launched the official
x86_64 Herdr 0.9.3 client (release SHA-256 verified), operated Work through
Cloudflare, detached with status 0 and restored its terminal. The service was
uninstalled afterward. These Intel binaries ran under Rosetta; the target Herdr
server was native arm64. Physical Intel Mac acceptance remains outstanding.

The existing 36 Rust tests and strict Clippy passed after introducing service
status and safe service-directory handling. No additional low-value template
snapshot tests or e2e suite were added; launchd was exercised directly.
