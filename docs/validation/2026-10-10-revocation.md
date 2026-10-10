# Directional revocation acceptance — 2026-10-10

Scope: issue #8 and the revoke/regrant race requirements from #7. Real
Herdr 0.9.3 clients, macOS arm64, and the restricted development Cloudflare
deployment were used. Worker version: `7677faea-359f-474e-a50c-85d8cb04fcb8`.
This is manual acceptance evidence, not an e2e test suite or release-platform
certification.

## Verified behavior

- B attached to A's Work and Build targets while C attached to A's Ops target
  and A attached in the reverse direction to B's Personal target.
- Revoking A's incoming B grant committed version 2. The first state snapshot
  detecting the commit, 0.232 seconds after starting the command, already
  found both native reservation locks available. The two B clients exited
  within 0.414 and 0.558 seconds of command start. The CLI, including cloud
  acknowledgement, completed in 0.832 seconds. These are observations from
  this run, not production latency guarantees.
- C's existing client and the reverse A-to-B client both remained usable and
  printed fresh distinct markers after the revocation. They detached normally.
  All four terminals restored their recorded termios state. Herdr servers
  and panes remained alive.
- A previously issued but unused B control ticket failed with
  `403 TICKET_INVALID`. B's new target query failed with `NOT_AUTHORIZED`.
- Reimporting the original retained invitation from B failed with
  `PAIR_INVALID`; its cached receipt did not recreate the revoked grant.
- A was stopped, C was revoked locally (version 3), and A was restarted.
  C's target query still failed with `NOT_AUTHORIZED`.
- A fresh invitation granted B version 4. B opened both role sockets but
  deliberately sent no inner TLS handshake. Revoking B again (version 5)
  closed the pending connection; the command completed in 0.490 seconds.

The initial four-client attempt exposed a Clap configuration panic before
revocation ran. The required `--origin` argument was moved onto each `peers`
subcommand; the scenario above was rerun successfully. The failed initial
attempt is not counted as revocation evidence.

## Necessary regression coverage

The persistent pairing module test exercises revoke, reload, receipt recovery
rejection, independent reverse grants, and regrant with a newer version.
The TLS context test includes a grant-version mismatch. The 31 Rust module
tests passed. No new e2e suite was added.

Local revocation is authoritative even when Cloudflare is unavailable. Cloud
ticket invalidation requires the publisher's state update to reach the Relay;
the CLI distinguishes a committed local revocation from a failed cloud sync.
