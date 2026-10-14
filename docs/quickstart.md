# Herdr Relay: first connection

Install Herdr and Herdr Relay on both computers. Relay does not pin a Herdr
release; the native client and server negotiate compatibility. See
[Herdr compatibility](herdr-compatibility.md). The source installation below
requires Rust 1.99 or newer:

```sh
curl -fsSL https://herdr.dev/install.sh | sh
cargo install herdr-relay --locked
```

Ensure the installed executables are on PATH. Alternatively, download a native
archive from [GitHub Releases](https://github.com/hackerchai/herdr-relay/releases/latest) or the [download site](install.md#download-site),
verify its checksum, extract it, and run `./install.sh`. Binary installation needs
neither Rust nor Node. No GitHub account is required.

## On the computer you want to access

Start Herdr and then detach with Ctrl-B, release, Q:

```sh
herdr
```

Start sharing:

```sh
herdr-relay serve
```

Use the arrow keys to move, Space to select sessions, and Enter to apply. Select
`default` for the session started by bare `herdr`. You can select several sessions.
Relay creates its default profile and installs/starts its background service.
Use the Copy action to copy the pairing link, or print it and copy it manually.
Closing this screen keeps sharing active.

Sleep protection is enabled while the server runs. Check `herdr-relay service
status` or **Sleep protection status** in the sharing menu. MacBook lid-close sleep
remains OS controlled; see [sleep protection and closed-lid setup](sleep-protection.md).

Login startup is enabled for a new server; press `a` during setup to toggle it.
Rerunning `serve` preserves existing selections and lets you change them. An
unchecked session stops being shared but its Herdr process continues running.

## On the client

```sh
herdr-relay pair
```

Paste the entire link and press Enter. The link is usable once and expires after
ten minutes; successful pairing persists. Choose the host and a session to enter
the native Herdr interface. Pairing grants access in one direction.

For every subsequent connection:

```sh
herdr-relay
```

`herdr-relay connect` opens the same menu. Hosts are named after the computer
(on macOS its computer name from Sharing settings); duplicate names receive
distinct aliases. Press `a` in the host menu to assign a clearer local name, or
`d` to remove a host you no longer use. `herdr-relay hosts list` and
`herdr-relay hosts remove NAME` do the same from scripts. Removing is local:
pairing again needs a new link, and the host can revoke this device with
`herdr-relay peers revoke`. No device ID, target ID, or `--profile` is required.

Offline, checking and access-revoked hosts stay in the host list when you press
Enter. Press `r` to check again; renaming and removing these hosts still works.
A host marked `relay token required` can be selected to configure its token.

## Keyboard help

| Key | Action |
| --- | --- |
| Up/Down or j/k | Move through a list |
| Space | Toggle a session in the sharing screen |
| Enter | Select or apply |
| / | Filter a host/session list |
| r | Refresh |
| ? or F1 | Help |
| Esc | Back |
| Ctrl-C | Exit |

While inside Herdr, use Herdr's keybindings. Ctrl-B, release, Q detaches and
returns to the Relay session picker. A busy session already has another Relay
client; disconnect that client before trying again.

## Background service and upgrades

```sh
herdr-relay service status
herdr-relay service stop
herdr-relay service start
herdr-relay service autostart on
herdr-relay service autostart off
herdr-relay update --check
herdr-relay update
```

macOS starts the user service at login. Linux can additionally start it at boot
and keep it after logout with `herdr-relay service autostart boot`, which may need
system authorization. Relay does not create Herdr sessions on reboot; once the
same Herdr session starts again, saved sharing resumes.

Updates come from the download site, falling back to GitHub Releases, without any login.
Relay checks for a new release once a day (when a menu opens and from the
background service) and shows `Update available: … Run herdr-relay update.` in
the menus and after they close. Change the interval with
`herdr-relay config set update-check 6h`, or turn it off with `off`.
Source installations can also update from crates.io:

```sh
cargo install herdr-relay --locked --force
```

The GitHub updater also checks the separately installed service executable, even
when the CLI is already current, including after a crates.io upgrade.
An active session defers its activation; stop/start the service when convenient,
or use `herdr-relay update --restart-service` to allow an immediate disconnect.
Updating Relay does not update Herdr, Codex, or Claude Code.
