# Ticket #5: one remote native target

2026-10-10, macOS 27 arm64, official Herdr 0.9.3, Rust 1.99.0.
Two isolated endpoint profiles ran on the same physical Mac. Both initiated
ordinary HTTPS/WSS to the controlled Cloudflare deployment; neither exposed
an inbound port. This does not establish cross-machine or cross-platform
acceptance.

Worker: `https://herdr-relay-development.hackerchai.workers.dev`
Version: `abafefea-516a-4701-b465-a001c556fb07`.
The deployment admits only three dedicated manual identities and includes the
SQLite Room migration. It is not the default public service.

## Observations

- A published only `/tmp/hr-live-native/runtime/herdr-client.sock`. B's target
  list returned the opaque ID, `online` and `work 中文` inside device TLS.
- A real official Herdr client connected through B's private IPC bridge. A
  unique `LIVE_978301619889125_中文_🙂` command result appeared in the host pane;
  its shell PID remained 23725. This was checked through the host's local API
  as well as the native client's rendered output.
- Resizing the client PTY from 24x80 to 40x120 changed pane geometry from
  `23 53` to `39 93`, accounting for Herdr chrome.
- A concurrent B attach returned `TARGET_BUSY`; target status remained `busy`
  and the first client continued operating.
- The active client remained connected for an additional 125 seconds. Cloud
  events showed Room `renew` RPCs without exceptions; the connection survived
  the initial 120-second lease. This is functional lease evidence, not a
  measurement of actual hibernation or billing.
- `Ctrl+B`, then `Q` returned exit 0. Exact before/after `stty -g` matched.
  The Herdr server and pane remained available after detach.
- After a later fresh invitation, C could query targets immediately after
  pairing success, confirming the grant mirror was ready before the receipt.
- Stopping only the dedicated Herdr server left A's Relay service online.
  B saw the target as `offline`; attaching returned `TARGET_OFFLINE`. The
  Relay did not restart Herdr or select another instance.
- An ungranted C made a correctly signed public access request and received
  HTTP 403 `NOT_AUTHORIZED`. B requested an unshared opaque target and received
  HTTP 409 `TARGET_OFFLINE`.

The initial live startup immediately after deployment returned a generic
Relay error. A retry without a Worker change succeeded; its root cause was
not captured, so deployment propagation is only a hypothesis. Known protocol
and internal error codes are now preserved by the CLI. Automatic startup
recovery belongs to ticket #9.

The initial manual PTY runner lacked a controlling terminal, preventing native
keyboard input. Giving its shell a controlling PTY fixed the harness. The
successful run waits for the rendered interface before sending a unique
marker, avoiding false positives from earlier pane history.

After the long-lived check, the publisher's control connection later ended
with `NETWORK_LOST`; subsequent requests returned `HOST_OFFLINE`. Restarting
the publisher restored access. This stage deliberately exits on network loss;
no automatic-recovery claim is made. Dedicated test servers and services were
stopped after these checks.

## Necessary tests and boundaries

29 Rust module tests passed; Clippy with warnings denied, formatting,
TypeScript checking and three shared-protocol TypeScript tests passed.
The new boundary tests cover:

- authenticated host reservation before native socket creation, exact control
  to native-byte transition, and release after disconnect;
- simultaneous 1 MiB transfers through bounded streams with slow readers;
- cancellation while the credit window is fully spent, without waiting for
  a peer credit that may never arrive.

The last case first failed, exposing a pump that could miss endpoint closure
while reads were gated on credit. A stream drop now signals cancellation
independently of credit. No automated e2e suite was added.

Names, sessions, paths and native contents stay inside endpoint TLS. Worker
state contains only opaque device/target/channel IDs, grant versions, role
credential digests, lease deadlines and credit counters. Runtime TLS and
wrong-context/identity/version rejection remain covered by the earlier TLS
and shared-codec tests. Further revocation races, pressure limits, persistent
multi-target management and network recovery are subsequent tickets.
