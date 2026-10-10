# Install Herdr Relay

Public distribution uses crates.io; GitHub binary archives remain private.
Install a published CLI release with Rust 1.99 or newer:

```sh
cargo install herdr-relay --locked
```

Binary archives contain a native CLI, installer, checksums and these instructions.
Binary installation needs neither Rust nor Node. The native Herdr **0.9.3** client and an already running target
Herdr session are required separately; Relay never installs or launches a server.
Use `--herdr /absolute/path/to/herdr-0.9.3` when connecting. For a bounded server,
use the [custom Herdr build](../patches/herdr/README.md), which remains compatible
with official 0.9.3 clients. Installing Relay does not retrofit existing servers.

Choose the archive for your OS and CPU. macOS Apple Silicon and Intel are distinct
builds; Linux candidates target musl on x86_64 and arm64. An archive is a build
artifact, not proof that its platform acceptance passed. Check the validation
records for the actual tested runtime. No minimum OS/distribution support claim
is made solely from successful compilation.

Check the `.tar.gz` against its adjacent checksum from the same trusted source:

```sh
shasum -a 256 -c herdr-relay-0.1.1-aarch64-apple-darwin.tar.gz.sha256
tar -xzf herdr-relay-0.1.1-aarch64-apple-darwin.tar.gz
cd herdr-relay-0.1.1-aarch64-apple-darwin
./install.sh
```

On Linux, `sha256sum -c` is equivalent. Checksums detect corruption; they are not
a publisher signature. The installer verifies its bundled files and writes only
`~/.local/bin/herdr-relay`. Set `HERDR_RELAY_INSTALL_DIR` to another absolute
user-owned binary directory if needed. Add that directory to PATH yourself.
New installation directories use a private umask, including on systems whose
default is group-writable. Existing directory permissions are not changed.
It does not edit shell startup files, install services, or change Gatekeeper
settings. These candidates are not notarized public macOS packages.

## macOS user service

Initialize the device, explicitly save shared targets, then install the service:

```sh
herdr-relay identity init
herdr-relay share add --name Work --session work
herdr-relay service install --origin https://your-relay.example
herdr-relay service start
herdr-relay service status
herdr-relay pair create
```

The same `--profile /absolute/private/profile` option selects the profile for all
commands. Services require a persistent profile outside temporary directories.
The service copies the current executable into the profile's private `service`
directory; deleting the extracted archive or closing the installing terminal
will not remove the running program. The launchd definition has a profile-specific
label under `~/Library/LaunchAgents`, private mode 0600, and absolute arguments.
Identity, grants, target definitions and the installed binary are user-only.
Only HOME and configured XDG directory variables are captured; shell credentials
and inherited Herdr overrides are not copied into the service definition.

Installation does not start the job. Start-at-login is opt-in:

```sh
herdr-relay service stop
herdr-relay service install --origin https://your-relay.example --start-at-login
herdr-relay service start
```

This is a login-session LaunchAgent, not a system daemon. Without the opt-in,
start it explicitly after login. Closing Terminal does not stop a started job;
logging out ends that user's GUI job. Automatic operation before GUI login is
not provided. Stopping an opted-in job stops this session; reinstall without the
flag to remove the next-login start setting. A fatal daemon exit requires another
`service start`; ordinary transient network failures reconnect within the daemon.

Repeated start does not replace a healthy process. Stop and uninstall are
idempotent and affect only this profile's job. Status distinguishes a loaded job
from a responding daemon and whether the Relay is online. Background output is
not retained; for a startup/configuration error, stop the job and run
`herdr-relay serve --relay --origin https://your-relay.example` to see diagnostics.
Do not run foreground and managed daemons for the same profile simultaneously.

## Linux user service

The same `service install/start/stop/status/uninstall` commands use
`systemctl --user`. Definitions live in `$XDG_CONFIG_HOME/systemd/user` or
`~/.config/systemd/user`, with private files and a profile-specific unit name.
No system-wide unit or privileged account is created. `--start-at-login` enables
the user unit under `default.target`; without it, start the unit explicitly.
The unit restarts failed processes with a five-second delay and a bounded start
rate. Explicit stop does not remove enablement for a future user-manager start.

`service status` reports `loginctl`'s current Linger setting when available.
With lingering disabled, the service's lifetime depends on login/session policy;
closing the last login session can stop the user manager. With lingering enabled,
the manager may remain active after logout. Relay does not enable lingering or
change system login policy. Any administrator-required linger change is an
explicit local administrator action. It is not promised by the installer.

Startup needs a working systemd user bus, normally supplied by the distribution's
login/session stack. If systemd or the user bus is absent, run foreground mode:

```sh
herdr-relay serve --relay --origin https://your-relay.example
```

An external supervisor may manage that exact command. Stderr goes to the user
journal for systemd jobs; use `journalctl --user -u UNIT` for diagnosis. Logs
contain service/error metadata, not terminal payload or invitation secrets.
A native socket and its parent directories must be owned safely and not writable
by other users. `UNSAFE_TARGET` can indicate Herdr directories created under a
group-writable umask; inspect the exact selected path and fix its permissions
locally before sharing. Relay never silently relaxes or rewrites those permissions.
Neither service mode nor foreground mode starts Herdr. Runtime validation records
must establish the supported distribution/kernel range; a musl build alone does
not establish universal Linux compatibility.

## Upgrade or remove

```sh
herdr-relay service stop
# Verify and run the new archive's install.sh.
herdr-relay service install --origin https://your-relay.example
herdr-relay service start
# Or remove only the background service:
herdr-relay service uninstall
```

Repeat `--start-at-login` during upgrade if desired. Install while loaded is
rejected, so an upgrade cannot silently replace an active job's configuration.
Stopping/uninstalling preserves identity, pairing grants and shared targets and
never terminates Herdr or its agents. Binary uninstall is removal of the exact
file installed in your chosen binary directory; private device data is preserved
unless you explicitly remove it. Keep a backup of the whole private profile for
recovery; never paste private keys or pairing invitations into reports.

## Building candidates

Maintainers can use `scripts/package.sh TARGET` with an installed Rust target
and the appropriate platform toolchain. It creates a local archive and SHA-256
manifest under `target/packages`; it does not publish a release. A cross-build
still needs actual runtime validation, and release licensing/operations decisions
remain prerequisites for public distribution.

Each candidate includes `BUILD-INFO.txt` (target, UTC time, compiler/toolchain,
build OS and commit when available) and `SOURCE-MANIFEST.sha256` for the exact
Rust/Worker/protocol/build-script inputs. Uncommitted checkouts are identified as
source snapshots, never assigned a fabricated commit. The script verifies those
inputs again after compilation and refuses a mixed-source candidate. The archive
manifest also authenticates the installer and provenance files. Preserve the
source snapshot alongside its candidate archives and validation record.
