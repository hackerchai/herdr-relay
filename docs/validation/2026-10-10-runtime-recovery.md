# Herdr and daemon runtime recovery — 2026-10-10

Scope: issue #10. Real Herdr 0.9.3 Work and Build sessions, two simultaneous
native clients, local Workers runtime and the restricted Cloudflare origin,
on macOS arm64. No e2e suite was added.

## Results

- Work's normal `server.stop` closed its old client and reported Work offline.
  Build stayed busy and usable without a reconnect. A human-equivalent manual
  restart of the same Work session restored access through the saved target
  ID and existing grant. Fresh distinct Work/Build markers reached the right
  panes. The Relay itself never started a Herdr process.
- Killing the newly started Work server left a stale socket pathname. The
  active/failed native connection marked that runtime offline. After manually
  starting Work again, the waiting client reattached automatically.
- Making Work's native socket mode 0666 made only Work offline and closed its
  connection. Restoring 0600 allowed recovery. Build's client remained active
  throughout these target-specific events.
- Restarting the publisher daemon restored both enabled definitions with a
  fresh boot. Both waiting clients reattached, and both panes accepted fresh
  markers after recovery.
- Disabling Work ended its client with `TARGET_NOT_SHARED`. Manually restarting
  Work did not republish it. A further daemon restart still exposed only Build.
- The previously revoked live B grant remained rejected after the dedicated
  Work server was restarted and the live publisher daemon started again.
  The live check returned `NOT_AUTHORIZED`.
- The final two-client run completed successfully. Work's disabled client
  exited 1, Build detached with exit 0, and separately recorded terminal
  settings were restored for both clients.

The first normal-server-stop run exposed an important distinction: Herdr sends
its own shutdown message, so its client can exit 1 before Relay sees stream EOF.
The connector now checks the authenticated target runtime after such an exit.
An offline or replaced runtime permits recovery; an unchanged available runtime
preserves the original client error. Successful detach never starts recovery.

A temporary harness initially allowed only eight seconds for the disabled
client to finish, ignoring accumulated retry backoff. The final run allowed the
configured recovery interval and confirmed the terminal error without changing
backoff or granting access. Harness terminal captures were also separated per
client to avoid shared output filenames during simultaneous runs.

## Implementation and checks

The daemon reconciles saved enabled targets every 500 ms. Device/inode and
socket ctime identify observed native generations; target IDs and grants remain
persistent. It marks retiring generations closed, cancels their operations
(including silent TLS handshakes), waits for owned native resources to drop,
then publishes the new in-memory generation. Other targets retain their objects
and channels. State synchronization is coalesced in a separate task so a slow
cloud request does not stop local administration.

Necessary public-module tests cover path replacement with an old native stream
still connected, pending TLS cancellation, stale-listener failure status,
permission changes, stable target identity, unrelated-target preservation and
disabled-target persistence. The 35 Rust tests passed before the final stale
socket adjustment; affected host/target tests and Clippy passed afterward.

No speculative native connection is opened for a status query. A stale socket
left by a server that had no active connection is recognized on an authorized
attach failure or a path/permission change; file metadata alone does not prove
that a listener is alive. This remains a status-observation limit, not permission
to start or select another Herdr session.
