# Linux candidates and user service — 2026-10-10

Issue: [#14](https://github.com/hackerchai/herdr-relay/issues/14). Both native CLI
architectures ran with their matching Linux kernels in isolated Ubuntu 24.04.4
LTS VMs on the development Mac. No host directories were mounted, SSH agent
forwarding was not enabled and the active Docker context was not changed.

| Target | Runtime |
| --- | --- |
| aarch64-unknown-linux-musl | ARM64 kernel 6.8.0-117-generic, Apple VZ virtualization |
| x86_64-unknown-linux-musl | x86_64 kernel 6.8.0-117-generic, QEMU CPU emulation |

This verifies those Ubuntu/kernel combinations, not all Linux distributions or
minimum kernel versions. ELF inspection showed musl static builds with no ELF
interpreter dependency. The x86_64 CLI also passed a supplemental Rosetta-on-Linux
native-client run, but the main x86_64 lifecycle result used an x86_64 guest
kernel. It is not physical Intel/AMD hardware performance evidence.

The ARM VM built both candidates using Rust 1.99.0, locked dependencies and an
x86_64 cross compiler. The separate x86_64 VM installed the checksum-verified
archive and ran without a Rust/Node build toolchain. Both official Herdr 0.9.3
architecture assets were checked against their upstream release SHA-256 values.

Each guest got a separate identity/profile, native Herdr server and explicitly
shared LinuxWork target. Both completed fresh, independently directed pairing
through the real Cloudflare development Relay with macOS endpoints. The
x86_64 host also ran the x86_64 native Herdr server. For each architecture:

- A macOS native client operated the Linux server, and the Linux native client
  operated the isolated macOS Work server. Pane reads on the actual servers
  confirmed typed markers. Unicode input was exercised.
- Native detach returned 0 and restored PTY termios. The isolated client config
  had completed/disabled Herdr's first-run onboarding before measuring input.
- Real `systemctl --user` install/start/status/stop/uninstall passed. Repeated
  start kept the same process, install while active failed, and the service's
  parent was the user manager rather than the installing SSH command.
- Repeated stop preserved Herdr. Restart resumed the existing macOS connection
  without pairing again. Input after reconnect reached the same native target.
- Repeated install while stopped and opt-in enablement under `default.target`
  passed. Repeated uninstall was idempotent; no job remained and exact identity,
  grant and target file hashes were preserved.

Both guests already had `Linger=yes` under Colima. Status reported this setting;
Relay did not enable it. No claim is made about distributions with lingering
absent or disabled. A missing user bus was simulated by pointing runtime/bus
variables to nonexistent paths: service installation returned `SERVICE_MANAGER`
and directed the user to foreground mode. A foreground publisher independently
completed real remote target discovery, native operation and normal detach.

The guest's umask 0002 exposed two setup issues. Existing unit-test fixtures were
changed to create private directories/sockets explicitly; production path checks
were retained. All 36 then-existing Rust tests passed under Linux with that umask.
Herdr-created test directories with group-write permissions were rejected as
unsafe; only those dedicated test directories were corrected. The installer
now sets umask 0077 for newly created installation parents, avoiding an unsafe
new `~/.local` directory. Existing user directories are not silently chmodded.

The x86_64 VM required the additional Lima guest-agent package. Homebrew also
upgraded its Lima/QEMU and related library dependencies; these are host test
infrastructure changes, not Relay runtime requirements. Docker provisioning in
that VM reported a failure after Linux/SSH/systemd were ready; no Docker runtime
was needed for the package/service acceptance. No permanent e2e suite was added.

After the installer fix, the x86_64 candidate was rebuilt, checksum-verified and
installed into a completely new nested directory under umask 0002. Both new
parent directories were mode 0700, and identity initialization beneath them
succeeded. The later origin-switch change also passed all 37 Rust tests in the
ARM64 Linux VM; both musl candidate packages were rebuilt from that source.
