# Herdr Relay

Herdr Relay is a Rust CLI and TypeScript Cloudflare Worker for accessing
explicitly shared Herdr sessions through the native client. It supports encrypted
accountless pairing, multiple shared targets, exclusive instance access,
revocation, reconnect and background user services. The default endpoint is
`https://herdr-relay.hackerchai.com`; use `--origin` to select a self-hosted Relay.

The project uses MIT licensing. The source repository stays private; the public crates.io package
contains CLI and Worker source plus self-host materials. Four-platform binaries
and anonymous updates are available from the
[public release repository](https://github.com/hackerchai/herdr-relay-releases/releases). [Release readiness](docs/release-readiness.md)
records publication and acceptance evidence.

Install the latest [public crates.io release](https://crates.io/crates/herdr-relay)
with Rust 1.99 or newer:

```sh
cargo install herdr-relay --locked
herdr-relay --version
```

Install the official [Herdr 0.9.3](https://github.com/herdrdev/herdr/releases/tag/v0.9.3)
client separately on both computers. Relay connects to an already running Herdr
session; it does not start the server. The default public Relay needs no
Cloudflare account. See the [installation guide](docs/install.md) for services
and [self-hosting guide](docs/self-hosting.md) for your own Cloudflare deployment.

The [release record](docs/validation/2026-10-10-release-0.2.1.md) identifies the
four-platform assets, production deployment and actual acceptance coverage.

For the bounded native server, use the [Herdr 0.9.3 compatibility patch](patches/herdr/README.md).
An unmodified upstream server still has its original unbounded control queue.
The maintainer accepted GUI testing; the [candidate record](docs/validation/2026-10-10-candidates.md)
retains actual OS, Rosetta and VM coverage.

Requirements are in [REQUIREMENTS.md](REQUIREMENTS.md), decisions are in
[docs/adr](docs/adr/README.md), and implementation work is tracked in
[GitHub Issues](https://github.com/hackerchai/herdr-relay/issues).

## Quick start

On the host, start Herdr normally, then detach with Ctrl-B followed by Q:

```sh
herdr
herdr-relay serve
```

Select the sessions to share with Space and press Enter. Relay initializes its
state, installs/starts the background service, and offers a pairing link to copy.
New servers start at login by default; press `a` in setup to change this setting.

On the client:

```sh
herdr-relay pair
```

Paste the complete link. Choose the hostname and Herdr session in the terminal
menu. On later visits, just run:

```sh
herdr-relay
```

This is equivalent to `herdr-relay connect`. No profile, device ID, or target ID
is needed. Duplicate hostnames receive distinct local aliases; press `a` in the
host menu to rename one. Press `?` or F1 for help, `/` to search, `r` to refresh,
and Esc to return. Detaching from Herdr returns to the session menu.

```sh
herdr-relay service status
herdr-relay service stop
herdr-relay service start
herdr-relay service autostart off
herdr-relay update
```

On Linux, `service autostart boot` enables user lingering for startup before login
and continued operation after logout, subject to the system's authorization
policy. macOS autostart runs at user login. Neither mode starts Herdr itself.

`update` checks complete stable GitHub Releases, verifies the selected archive,
and updates the CLI and installed server. The default public release channel
requires no GitHub login. Source installations can also use
`cargo install herdr-relay --locked --force`.
Active sessions defer the server restart; `update --restart-service` explicitly
allows disconnecting them. See the [simple walkthrough](docs/quickstart.md).

## Development

Building the CLI requires Rust/Cargo 1.99 or newer. Native release archives need
neither Rust nor Node at runtime. Platform acceptance and its Rosetta/VM limits
are recorded separately from CI build results.

From the repository root:

```sh
cargo check --locked
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

`Cargo.lock` records the dependency versions. A first build needs access to
crates.io. Keep necessary tests around meaningful behavior and regression risks;
test placement does not require another maintainer confirmation. By the
maintainer's instruction, do not add e2e tests at this stage; use the
[manual acceptance checklist](docs/manual-acceptance.md) for the overall workflow
and record the outcome.

## Install and manage a user service

See [installation and service commands](docs/install.md). Local archives
include checksums and need no Rust or Node at runtime. `service install/start/stop/status/uninstall` manages a private,
profile-specific macOS LaunchAgent or systemd user unit. Interactive server setup enables login startup by default; explicit service install retains its opt-in flag.

## Local native-client bridge

Start a Herdr 0.9.3 session yourself, then use an interactive terminal:

```sh
cargo run --locked -- local --session work --herdr /absolute/path/to/herdr-0.9.3
```

Use `--session default` to explicitly choose the default session. The resolver
uses `$XDG_CONFIG_HOME/herdr`, or `$HOME/.config/herdr`. An explicit
`--herdr-config-dir /absolute/path` overrides that directory. For a custom
native socket, use `--socket /absolute/path/herdr-client.sock` instead of
`--session`. A missing session never starts Herdr or falls back to another one.

The launcher removes inherited `HERDR_*` overrides from its child, supplies a
private bridge socket and uses the official remote keybinding mode. Normal
detach is Herdr's default `Ctrl+B`, then `Q`. Only the current client is cleaned
up; Herdr and its panes keep running. Each instance permits one Relay attach,
including a pending attach. A second returns `TARGET_BUSY`.

Errors distinguish missing/unsafe targets, incompatible client version,
startup failure, handshake timeout and cancellation. No Relay control message
is written into the native byte stream. See the
[manual acceptance record](docs/validation/2026-10-10-local-cli.md).

## Device identity and local encrypted diagnostic

```sh
cargo run --locked -- --profile /absolute/private/profile-a identity init
cargo run --locked -- --profile /absolute/private/profile-b identity init
cargo run --locked -- --profile /absolute/private/profile-a identity show
cargo run --locked -- --profile /absolute/private/profile-a identity diagnose \
  --peer-profile /absolute/private/profile-b
```

The default profile is `$XDG_STATE_HOME/herdr-relay`, or
`$HOME/.local/state/herdr-relay`. Profiles must be private directories; missing
directories are created with mode 0700. Keys are stored with mode 0600. Losing
a key never silently creates a replacement identity. The diagnostic uses a
local socket pair and does not grant access to either device. See the
[identity and TLS acceptance record](docs/validation/2026-10-10-device-tls.md).

## Local pairing workflow

With two initialized profiles, start the publisher in a separate terminal:

```sh
target/debug/herdr-relay --profile /absolute/profile-a serve --foreground \
  --origin https://local-pair.invalid
```

Create and import a one-time invitation using stdin:

```sh
target/debug/herdr-relay --profile /absolute/profile-a pair create |
  target/debug/herdr-relay --profile /absolute/profile-b pair accept \
    --local-socket /absolute/profile-a/daemon.sock
target/debug/herdr-relay --profile /absolute/profile-a devices \
  --origin https://local-pair.invalid
target/debug/herdr-relay --profile /absolute/profile-b devices \
  --origin https://local-pair.invalid
```

Without piped input, `pair accept` prompts for a hidden paste. Never put the
invitation in command arguments. `pair revoke <invitation-id>` invalidates an
unused invitation. Invitations expire after ten minutes; an interrupted
redemption can recover its original result for 24 hours by importing the same
invitation from the same profile. Grants persist until explicitly revoked.
Pairing grants B access to A's explicitly shared instances; it does not publish
local resources or grant A access to B. The local transport currently serves
diagnostics; the origin identifies the trust scope and is not dialed. See the
[manual pairing record](docs/validation/2026-10-10-local-pairing.md).

For pairing through the Worker/DO runtime, see the
[Relay development guide](relay/README.md). Both endpoints initiate their
connections; the publisher does not expose an inbound network port.

## Revoke a paired device

On the publisher, list or revoke incoming access:

```sh
target/debug/herdr-relay --profile /absolute/profile-a peers list \
  --origin https://your-relay.example
target/debug/herdr-relay --profile /absolute/profile-a peers revoke <device-id> \
  --origin https://your-relay.example
```

Revocation persists locally before connected sessions are cancelled and the
Relay is updated. It closes that peer's pending and active channels across
all shared targets; other peers and independently granted reverse access keep
working. Offline revocation takes effect when the service next starts. A saved
invitation receipt cannot restore a revoked grant: access requires a fresh
invitation. If Relay synchronization fails, `REVOKED_LOCALLY` means the local
revocation has already committed. See the
[revocation acceptance record](docs/validation/2026-10-10-revocation.md).

## Persistent shared targets

After both profiles are paired at the same HTTPS Relay origin, explicitly add
already running sessions on A, then start its service:

```sh
target/debug/herdr-relay --profile /absolute/profile-a share add \
  --session work --name Work
target/debug/herdr-relay --profile /absolute/profile-a share add \
  --session personal --name Personal
target/debug/herdr-relay --profile /absolute/profile-a serve \
  --origin https://your-relay.example --relay
```

Use `--socket` instead of `--session` for an explicit native socket, and
`--herdr-config-dir` for a different Herdr configuration root. `share add`
prints a stable opaque target ID; names and local paths remain on the endpoints.
From B, with A's device ID:

```sh
target/debug/herdr-relay --profile /absolute/profile-b targets \
  --origin https://your-relay.example --host <device-id>
target/debug/herdr-relay --profile /absolute/profile-b connect \
  --origin https://your-relay.example --host <device-id> --target <target-id> \
  --herdr /absolute/path/to/herdr-0.9.3
```

`status` is an alias for `targets`. The list is exchanged inside device TLS.
Only one pending or active Relay client can occupy an instance. A second
returns `TARGET_BUSY` without replacing the first. Native detach is `Ctrl+B`,
then `Q`; Herdr and its panes remain running. A network failure closes the old
client and starts recovery with a new channel and TLS handshake. The terminal
shows a retry delay; press Ctrl-C to cancel. Offline keystrokes are discarded,
including pending input before the native client restarts. A busy target,
revoked grant, identity/version failure, or removed target ends recovery.
The publisher service also reconnects while keeping local administration
available. Recovery never starts a Herdr server. See the
[network recovery record](docs/validation/2026-10-10-reconnect.md).
Use `share list`, `share disable <id-or-exact-name>`, `share enable` and
`share remove` to manage saved definitions. Disabling or removing a target
closes only its channels; enabled targets and grants remain intact. Disabled
targets stay disabled after a service restart. Duplicate session/socket aliases
are rejected even when the original target is disabled. See the
[remote acceptance record](docs/validation/2026-10-10-remote-native.md).

The service observes the saved sessions or socket paths. When an enabled
instance restarts, it closes the old runtime's channels and restores sharing
with the same target ID and current grants. Waiting clients attach to the new
runtime; the service never starts Herdr. Restarting one target leaves the others
connected. See the [runtime recovery record](docs/validation/2026-10-10-runtime-recovery.md).

## Isolated upstream compatibility probe

The initial native protocol baseline is the official Herdr **0.9.3** release.
The probe needs Python 3 and an explicitly supplied Herdr binary. It uses
temporary, isolated configurations and only cleans up processes it creates.
It does not replace an installed Herdr or connect to an existing user session.

For macOS Apple Silicon, download and verify the pinned official artifact:

```sh
mkdir -p /tmp/herdr-relay-tools
gh release download v0.9.3 --repo herdrdev/herdr \
  --pattern herdr-macos-aarch64 --dir /tmp/herdr-relay-tools --skip-existing
shasum -a 256 /tmp/herdr-relay-tools/herdr-macos-aarch64
```

The expected SHA-256 is:

```text
5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157
```

Only after the checksum matches:

```sh
chmod u+x /tmp/herdr-relay-tools/herdr-macos-aarch64
python3 scripts/probe-herdr-0.9.3.py /tmp/herdr-relay-tools/herdr-macos-aarch64
```

This checks upstream native protocol behavior. It does **not** test a real TUI,
the Rust CLI bridge, device trust, or a Cloudflare connection. In particular,
the probe's use of multiple native clients does not change the product rule:
one active Relay client per Herdr instance. See the
[compatibility report](docs/herdr-compatibility.md) and
[bootstrap evidence](docs/validation/2026-10-10-bootstrap.md).

Keep credentials outside the repository. Build output, local environment files,
and Wrangler development secrets are ignored by Git.

Self-hosting and explicit Relay-origin switching: [deployment guide](docs/self-hosting.md).
