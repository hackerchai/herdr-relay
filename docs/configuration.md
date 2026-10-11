# Configuration

Herdr Relay works without any configuration: the defaults use the hosted Relay,
the `herdr` executable on your `PATH` and a private profile in your home
directory. When you want to change one of these permanently, such as pointing
every command at a [self-hosted Relay](self-hosting.md), save it in
`config.toml` instead of repeating a flag or exporting a variable in every shell.

## Where the file lives

Herdr Relay reads the first location that applies:

1. `$HERDR_RELAY_CONFIG`, an explicit path to the file;
2. `$XDG_CONFIG_HOME/herdr-relay/config.toml`;
3. `~/.config/herdr-relay/config.toml`.

A missing file is the same as an empty one. `herdr-relay config show` prints the
path in use. Herdr Relay creates the file and its directory on the first
`config set` and always writes it atomically with mode `0600` (readable only by
you), because it can hold access tokens.

## Precedence

Each setting is resolved in this order, and the first value found wins:

1. a command-line flag, such as `--origin`;
2. its environment variable, such as `HERDR_RELAY_ORIGIN`;
3. `config.toml`;
4. the built-in default.

The file therefore sets your defaults, and a flag or variable still overrides it
for a single command or shell.

## Managing settings

Use the `config` command rather than editing the file by hand; it validates each
value before saving it.

```sh
herdr-relay config show                 # the file path and its settings (tokens hidden)
herdr-relay config set KEY VALUE        # save a setting
herdr-relay config unset KEY            # remove it so the variable or default applies
```

For example:

```sh
herdr-relay config set origin https://relay.example.com
herdr-relay config set herdr /opt/homebrew/bin/herdr
herdr-relay config set update-check 6h
herdr-relay config set direct-port 47121
herdr-relay config show
```

```text
/Users/you/.config/herdr-relay/config.toml
origin = https://relay.example.com
herdr = /opt/homebrew/bin/herdr
update-check = 6h
direct-port = 47121
```

You can also edit `config.toml` directly. Unknown keys and invalid values are
rejected with a `CONFIG_INVALID` error naming the file, and every command
refuses to run until it is fixed, so a typo cannot be silently ignored.

## Settings

| Key | Environment | Default | Meaning |
| --- | --- | --- | --- |
| `origin` | `HERDR_RELAY_ORIGIN` | `https://herdr-relay.hackerchai.com` | The Relay to use |
| `profile` | `HERDR_RELAY_PROFILE` | `$XDG_STATE_HOME/herdr-relay` or `~/.local/state/herdr-relay` | Where identity, pairings and shared sessions are kept |
| `herdr` | `HERDR_RELAY_HERDR` | `herdr` | The Herdr executable |
| `update-check` | `HERDR_RELAY_UPDATE_CHECK` | `1d` | How often to check for updates, or `off` |
| `update-url` | `HERDR_RELAY_UPDATE_URL` | `https://herdr-relay-dl.hackerchai.com` | The release download site tried first |
| `update-repository` | `HERDR_RELAY_UPDATE_REPOSITORY` | `hackerchai/herdr-relay` | GitHub owner/repository for releases |
| `direct` | `HERDR_RELAY_DIRECT` | `on` | Whether a host accepts direct LAN connections |
| `direct-port` | `HERDR_RELAY_DIRECT_PORT` | `47120` | The TCP port for direct connections |
| `token` | `HERDR_RELAY_TOKEN` | none | A self-hosted Relay's access token, saved per origin |

### `origin`

The exact `https://` origin of the Relay, with no path or trailing slash, for
example `https://my-herdr-relay.example.workers.dev`. Set it on both the host and
the client when you use a [self-hosted Relay](self-hosting.md), ideally before
the first pairing. Pairings belong to one Relay; to move an existing profile, see
[background services](#background-services).

### `profile`

The private directory holding this device's identity, pairings, shared sessions
and the background service copy. It is separate from Herdr's own configuration.
Use a different profile to run an independent Relay identity on the same
account. A background service needs a persistent location outside temporary
directories.

### `herdr`

The Herdr executable used to open sessions. Set an absolute path when Herdr is
not on the `PATH` of every shell you use, for example
`/opt/homebrew/bin/herdr` or `~/.cargo/bin/herdr`.

### `update-check`

Relay checks for a newer release when a menu opens and from the background
service, and shows a one-line prompt; nothing is installed until you run
`herdr-relay update`. Use a number followed by `m`, `h` or `d` (at least one
hour), such as `6h` or `2d`, or `off` to stop checking. See
[interactive setup and updates](install.md#interactive-setup-and-updates) for details.

### `update-url` and `update-repository`

Where `herdr-relay update` downloads releases. The download site, an exact
`https://` origin with the same layout as the default one, is tried before
GitHub Releases. A GitHub `owner/repository` selects another release channel,
such as a fork, and skips the download site. Most users never change these.

### `direct` and `direct-port`

When both computers share a LAN or tailnet, the client connects to the host
directly, with the same end-to-end encryption, and falls back to the Relay
otherwise. These settings apply to the host: `direct` takes `on` or `off`
(`true`/`false` and `yes`/`no` also work), and `direct-port` is a TCP port from 1
to 65535. Allow that port on the host's firewall.

```sh
herdr-relay config set direct off       # always use the Relay
herdr-relay config set direct-port 47121
```

### `token`

A self-hosted Relay may require an access token from its operator. Tokens are
saved per Relay origin and sent only to that origin; the hosted Relay never
receives one and refuses to store one. Omit the value to type it with hidden
input, so it does not end up in your shell history:

```sh
herdr-relay config set origin https://relay.example.com
herdr-relay config set token            # for the configured origin
herdr-relay config set token --origin https://other.example.com
herdr-relay config unset token --origin https://other.example.com
```

A token is 16 to 512 visible ASCII characters. `config show` lists which origins
have one but never prints it. `HERDR_RELAY_TOKEN` overrides the saved token for
every self-hosted origin while it is set. See
[optional access token](self-hosting.md#optional-access-token).

## File format

`config.toml` uses the same kebab-case keys. Tokens live in a `[tokens]` table
keyed by origin:

```toml
origin = "https://relay.example.com"
herdr = "/opt/homebrew/bin/herdr"
update-check = "6h"
direct = true
direct-port = 47121

[tokens]
"https://relay.example.com" = "your-access-token"
```

In the file, `direct` is a TOML boolean (`true` or `false`), `direct-port` is a
number and every other value is a string.

## Background services

The background service started by `herdr-relay serve` reads the same
`config.toml` when it starts. When you install the service, Herdr Relay records
`HOME`, the XDG directory variables and `HERDR_RELAY_CONFIG` from your shell, so
the service finds the same file; other environment variables are not copied
into the service.

After changing a setting, restart the service so it takes effect:

```sh
herdr-relay service stop
herdr-relay service start
```

The service's Relay origin and profile are fixed when it is installed, and a
profile belongs to one Relay. Changing `origin` alone therefore fails with
`PROFILE_MISMATCH` instead of moving existing pairings; follow
[change origin explicitly](self-hosting.md#change-origin-explicitly) to switch an
existing profile to another Relay.

## Diagnostics

`HERDR_RELAY_TIMING=1` prints connection phase timings to stderr, or appends
them to the file it names. It is an environment variable only and has no
`config.toml` key.
