# Interactive CLI and server lifecycle

Accepted 2026-10-10 from the maintainer's requested UX and implementation goal.
The primary publisher workflow is `herdr-relay serve`: choose existing Herdr
sessions, apply the selection, and let Relay install/start its user service.
Clients paste a link into `herdr-relay pair`; bare `herdr-relay` and `connect`
open the same host/session browser. Ratatui with Crossterm owns the terminal
only while Relay menus are visible, yielding all input and output to Herdr
during native attach. Existing explicit commands remain available.

Hostnames and local aliases are display labels. Durable identities and target
identifiers remain internal protocol keys; renaming cannot redirect a grant.
A versioned pairing link carries the existing invitation and hostname as one
opaque token in an HTTPS fragment. This changes ADR-0011's presentation rule
that invitations never appear in URLs: the fragment is copied into the CLI and
is not sent to the web server. The landing page is informational, has no script,
and never redeems a grant. Existing identity verification, one-use expiration,
directional authorization and legacy invitations remain supported.

New server installations default to user-login startup, visibly configurable
in setup and service commands. Existing installations retain their preference.
macOS uses a LaunchAgent; Linux uses a user systemd unit and offers an explicit
linger option for boot/after-logout operation. This expands ADR-0013's user-service
policy on Linux; it does not add a macOS system daemon or start Herdr itself.
Service managers use an explicit unattended command so changing CLI defaults
cannot accidentally open a TUI from launchd or systemd.

GitHub remains private and crates.io remains public, as already authorized in
ADR-0013. The updater uses the caller's own GitHub credentials for private assets;
an explicit alternate release repository supports a later approved public binary
channel. No maintainer credential is embedded. A readiness manifest is published
only after all four native archives, crates.io publication, and production
deployment succeed. The CLI and copied service executable are upgraded together;
active sessions defer server activation unless an immediate restart is requested.
