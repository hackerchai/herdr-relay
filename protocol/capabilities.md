# Negotiated capabilities and the direct path

Added in 0.2.4. Every capability is optional and negotiated, so any mix of
older and newer CLIs and Workers keeps working with the
[v1 behavior](access-v1.md). Rust messages reject unknown fields; a peer
therefore never receives a field or message it did not ask for, except where an
older peer provably ignores it (noted below).

## Negotiation

Endpoints send `X-Relay-Features` on every relay HTTP request and WebSocket
upgrade, for example `stream-v2,prepared-join,control-authenticated`. Unknown
names are ignored; at most 16 names of `[a-z0-9-]{1,32}` are read. The Worker
answers the publisher's control upgrade with its own `X-Relay-Features`.

| Capability | Enabled when | Effect |
| --- | --- | --- |
| `stream-v2` | Publisher (on its control upgrade) and connector (on `/access`) both advertise it | Channel limits gain `window`; credits use their own message budget |
| `prepared-join` | The endpoint receiving the challenge advertised it | A join challenge is issued in advance |
| `control-authenticated` | Publisher advertised it and the Worker's control response lists it | Authentication is acknowledged on the control WebSocket |

## Stream v2 limits

Without `stream-v2` a channel keeps `{"bytes_per_second": R}` with `R` capped at
262144 and the 64 KiB credit window. With it, both endpoints receive the same
`{"bytes_per_second": R, "window": W}`:

| Var | Default | Allowed |
| --- | --- | --- |
| `CHANNEL_BYTES_PER_SECOND` | 1048576 | 16384–4194304 |
| `CHANNEL_WINDOW_BYTES` | 262144 | 65536–1048576 |

The burst is `max(W, 2R)` bytes. Data messages and credit messages each have a
300-message burst refilled at 150 messages per second, so traffic in one
direction cannot starve the acknowledgements of the other. The negotiated
window is persisted with the channel and survives Durable Object hibernation.

Endpoints send a buffered chunk as soon as the byte budget covers its actual
size; a one-byte keystroke never waits for 16 KiB of budget.

## Pre-issued join challenges

A `prepared-join` connector receives `join_challenge` in the `/access` and
`/pair` responses. A `prepared-join` publisher receives `join_challenge` in
the `access` and `pair` control events. Each is an ordinary access challenge
for `GET /v1/devices/{publisher}/channels/{channel}/{role}` whose payload hash
is the SHA-256 of that role's ticket, issued for the public key of the
already verified proof. The endpoint signs it only when origin, method, path,
public key and payload match exactly; otherwise, or when the join is rejected
with `ACCESS_INVALID`, it fetches a fresh challenge. Issuing failures omit the
field. This removes one round trip from each side of every channel.

## Control-socket authentication

A publisher with `control-authenticated` reports mutual authentication by
sending, on its control WebSocket:

```json
{"type":"authenticated","channel_id":"<32 hex>","boot_id":"<its boot>"}
```

The Durable Object applies the same checks as
`POST /v1/devices/{id}/channels/{channel}/authenticated` and replies
`{"type":"authenticated","channel_id":…}` or
`{"type":"authentication_failed","channel_id":…,"code":…}`. Any other message
still ends the control connection. Publishers overlap this acknowledgement
with the endpoint exchange; native bytes flow only after it succeeds. A
listing that completed before a late acknowledgement is not failed.

## Host and client information

On a control channel the publisher sends, right after `Targets`:

```json
{"result":"host_info","hostname":"…","direct":{"listener_id":"…","port":47120,"addresses":["192.168.1.20"]}}
```

`direct` is `null` when the host has no direct listener. Older clients send
`Ack` and close without reading it. After `Ack`, a newer client sends
`{"hostname":"…"}`; older hosts have already closed. Both travel only inside
the endpoint TLS channel. The client refreshes the host's saved name (an alias
the user never customized follows it), and the host records the client's name
for `peers list`.

## Direct path

A host's service listens on TCP (default port 47120). `HERDR_RELAY_DIRECT=off`
disables it, `HERDR_RELAY_DIRECT_PORT` changes the port, and a taken port
falls back to a free one. It accepts only peers from private, CGNAT/tailnet,
ULA, link-local or loopback addresses and at most eight concurrent
handshakes.

It advertises private IPv4, CGNAT (`100.64.0.0/10`) and ULA addresses, at most
eight. A client tries addresses on one of its own subnets first, then tailnet
addresses when it has a tailnet interface itself. Attempts start 250 ms apart;
each TCP connect waits at most 700 ms and the handshake at most 3 seconds.

The connection carries TLS 1.3 directly over TCP. The host's verifier accepts
only a client certificate pinned by a current grant, and TLS 1.3 encrypts that
certificate, so no device identity is visible on the network. The client pins
the host certificate from its grant as usual. The client then sends its
channel context; the host checks that the origin, both identities, the current
grant version, a non-pairing purpose and `boot_id` equal to the advertised
`listener_id` all match, and echoes it. Targets, reservations, revocation and
single-client attach then behave exactly as on the relay.

Any direct failure falls back to the relay within the same operation and
suppresses direct attempts to that host for 10 minutes unless the host
advertises a different endpoint. A direct attach leaves no relay channel to
release on reconnect.
