# Cloudflare pairing: local and live acceptance

2026-10-10, macOS arm64, Node 24.21.0, Wrangler 4.149.0,
workerd 1.20261006.1, TypeScript 6.0.3. Issue #4 local and controlled live
Cloudflare deployment verification. Both endpoint profiles run on this Mac;
this is not a two-machine or cross-platform acceptance record.

Two new Rust profiles connected to local Wrangler through explicit loopback
transport, keeping `https://relay.local.invalid` as their signed trust origin.
The publisher's control WebSocket became online, registered an invitation
route, opened its independent pair WebSocket and exchanged authenticated TLS
with the connector. B persisted A's grant and displayed success. B's retry
recovered grant version 1. C importing the same invitation failed.

A local Worker reload closed the publisher's control connection. The service
exited with `NETWORK_LOST`; a connector retry received `HOST_OFFLINE`. Restarting
the publisher created a new boot generation and B recovered the same persisted
grant. No Herdr process was needed for these checks.

Additional manual checks against the actual Workers/DO runtime:

- A valid access proof reached the offline-host check; replay of the same
  proof was rejected with HTTP 401.
- Wrong-device and wrong-role ticket attempts returned HTTP 403.
- A consumed ticket returned HTTP 409, while the original channel continued
  forwarding its exact binary frame.
- A frame with the wrong channel ID closed both roles.

Necessary tests cover shared Rust/TypeScript signature bytes, actual WebCrypto
signature rejection, strict control encoding and shared frame boundary cases,
including offsets above JavaScript's safe-integer range and maximum u64.
The full Rust suite (26 module tests), Clippy with warnings denied, TypeScript
checking and three TypeScript module tests passed. No e2e suite was added.

The controlled development deployment is
`https://herdr-relay-development.hackerchai.workers.dev`, version
`ff21610d-bf53-487b-bd63-b00867e8dd51`. It admits only the three dedicated manual
profiles. Both endpoints initiated ordinary HTTPS/WSS connections with normal
public certificate validation, followed by device-pinned inner TLS.

The first live pairing succeeded. Subsequent attempts exposed a reply-close
race: closing the publisher WebSocket could discard an in-flight TLS response.
A necessary regression test first reproduced early closure. The publisher now
waits for an encrypted request-ID acknowledgement; the connector persists a
successful receipt before acknowledging it. Both send TLS close_notify before
closing. The regression test passed. Three consecutive live B retries recovered
the same grant version, and two interleaved C attempts returned PAIR_INVALID.
Stopping A made a B attempt return HOST_OFFLINE; restarting A recovered its
persisted grant. No Herdr process was needed.

`wrangler deploy --dry-run` and the live deployment both accepted the AccessGate
and DeviceRelay SQLite DO bindings. This does not establish actual hibernation
billing, public-service readiness, or four-platform support. The supplied
Global API Key authenticated with the user-provided account email. Neither
credential was written to tracked files or this record.
