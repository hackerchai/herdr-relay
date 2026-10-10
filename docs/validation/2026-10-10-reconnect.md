# Network recovery acceptance — 2026-10-10

Scope: issue #9. Real Herdr 0.9.3 clients on macOS arm64, local workerd with a
temporary TCP fault proxy, and the restricted live Cloudflare deployment.
Worker version: `57dfe3ee-351b-4c60-b2c2-ea2864d0423b`. These are manual
observations, not an e2e suite or a four-platform release claim.

## Completed scenarios

- Abrupt connector disconnection followed by repeated connection refusal:
  the native client exited, recovery remained cancellable, and the same
  saved target reattached automatically with a different channel ID.
- A command typed while offline was neither echoed nor executed after
  recovery. A unique file it would have created remained absent. Fresh
  commands after recovery reached the intended pane.
- Publisher connection refusal: local service stayed alive and published
  a fresh boot after connectivity returned; the foreground client recovered.
- A connector role upgrade was dropped during setup. Recovery cancelled
  that attempt and attached successfully in 1.61 seconds in this local run.
- Repeated HTTP 429 responses with `Retry-After: 3`: the client remained in
  recovery and observed five rate-limit responses over the initial 20-second
  observation, then recovered after admission resumed.
- Another authorized client acquired the target while the first was offline.
  The first exited with `TARGET_BUSY` when connectivity returned. The new
  holder stayed usable; there was no waiting to take over or preemption.
- Ctrl-C while offline produced `CANCELLED`, exit 130 and restored termios.
  The temporary harness initially expected generic error exit 1; inspecting
  the CLI's documented cancellation exit and the actual terminal result
  corrected that harness expectation, not the implementation.
- Live Cloudflare publisher restart while a client occupied Build: the client
  waited, established a new channel and printed `LIVE_RECONNECTED_NO_REPLAY`.
  Its offline command did not execute. Native detach returned 0; terminal
  flags were restored and the Herdr server/pane remained alive.
- Live release endpoint: another device received `403 NOT_AUTHORIZED`;
  the owner and repeated owner release succeeded. Reusing the cancelled
  channel ID failed with `409 CHANNEL_CONFLICT`. Cancelling an unconfirmed
  open also prevented its later open from creating a channel.

The first half-open run exposed an overly coarse heartbeat deadline: checking
75 seconds of silence only on 25-second ticks took 97.45 seconds in the
follow-up measurement. It still recovered without replay and detached cleanly.
The independent silence timer detected the same fault in 73.16 seconds after
injection (75 seconds since the last received data). The client then recovered,
the offline command did not execute, normal detach returned 0, and termios was
restored.

A separate live run revoked B while A was stopped and B was waiting to
reconnect. After A restarted with persisted revocation version 7, B exited with
`NOT_AUTHORIZED` rather than retrying indefinitely. Its offline command was not
executed and termios was restored.

## Verification boundary

The 34 Rust module tests passed after the initial recovery implementation.
The affected retry, terminal, transport, stream and TLS module tests were rerun
after error-classification and heartbeat changes. TypeScript's three shared
protocol/cryptography fixture tests and its type check passed. Rust Clippy
passed. No permanent fault-proxy or end-to-end suite was added.

Recovery retries transient transport/offline/rate-limit errors. It stops on
permission, identity, protocol, incompatible-client, disabled/removed-target
and busy errors. It never starts Herdr and never resumes old native bytes.
The native inode lock can conservatively report busy while an old host socket
is still closing; recovery does not queue behind an unconfirmed owner.

## Follow-up found during user-service validation

A real launchd restart exposed a narrow startup race: the control WebSocket
existed before the new boot's grant mirror was synchronized, so an already
paired connector could receive terminal `NOT_AUTHORIZED` during recovery.
DeviceRelay now persists a readiness flag on the control attachment. Before
the initial `/state` transaction, access returns retryable `HOST_OFFLINE`;
normal authorization applies after that transaction. Missing readiness on an
older development attachment closes conservatively and reconnects. A managed
service stop/start then recovered the existing native client without re-pairing.
The publisher's local status separately reports ready only after state sync;
the boot generation remains available earlier for stale-update checks.

## Final-candidate revocation after repeated faults

The x86_64 candidate under Rosetta was revoked after several short-lived
recoveries. Its native reservation was already released when local revocation
was first observed, 54 ms after command start. The foreground supervisor then
waited its existing 23.7-second backoff before learning that fresh access was
denied and exiting 1. Terminal settings were restored. Backoff resets only after
a stable connection, so quick recoveries do not reset this delay. A delayed
frontend error must not be reported as continued data access; Ctrl-C remains
available during that offline interval. Detailed results are in the
[candidate record](2026-10-10-candidates.md).
