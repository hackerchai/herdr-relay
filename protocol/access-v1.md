# Relay access proof and pairing routing

This implemented development protocol complements the endpoint TLS protocol.
It is not a declaration of release wire compatibility. Shared signature and
frame fixtures are in `fixtures/`; both Rust and TypeScript consume them.

The configured Relay origin is a canonical HTTPS origin. Production endpoints
dial it using ordinary HTTPS/WSS certificate validation. Local Workers testing
requires an explicit `--relay-local-url http://127.0.0.1:8787` override; that
override accepts only a literal loopback address and does not alter the signed
origin or disable inner TLS. Inner TLS remains mandatory in both modes.

## Device admission

`POST /v1/challenge` accepts version, origin, method, path, device ID, raw
Ed25519 public key and SHA-256 of the operation payload. The server checks that
the device ID is SHA-256 of RFC 8410 Ed25519 DER SPKI, then returns a random
128-bit nonce and a decimal-string Unix expiry 60 seconds in the future.

The signed UTF-8 bytes are these newline-separated values, including a final
newline, with no other whitespace:

```text
herdr-relay-access-v1
origin
method
path
device_id
nonce
expires_at
payload_sha256
```

The Ed25519 signature and full challenge are sent as compact JSON in the
`X-Relay-Proof` header. Hex fields are lowercase. Expiry is a positive decimal
string without leading zeroes and no greater than JavaScript's safe-integer
limit. Only defined method/path combinations are accepted. JSON control
envelopes use compact JSON, canonical number spelling and no insignificant
whitespace; duplicate keys, unknown fields, malformed UTF-8 and bodies over
8 KiB are rejected. HTTP body reads have a ten-second deadline.

The Worker binds the proof to the actual method, path and payload before
calling the device AccessGate DO. That DO checks origin, expiry, key and
signature, then atomically compares and consumes its stored nonce. Invalid
proofs cannot enter a Device DO. A consumed proof cannot be reused even if the
requested operation failed. Request a fresh challenge for every HTTP/WSS
operation. Limits currently include four pending challenges per device and
120 challenges/minute per source-IP digest. These development limits are not
the final public-service abuse policy.

## Routes and payload binding

All paths below start with `/v1/devices/{publisher_device_id}`.

| Method and suffix | Signed payload | Behavior |
|---|---|---|
| GET `/control` | `X-Relay-Boot` value | Publisher's online WSS, one active control connection |
| POST `/invitations` | Exact HTTP body bytes | Register opaque invitation ID and expiry; publisher only |
| POST `/pair` | Exact HTTP body bytes | Connector requests a new channel for an invitation route |
| POST `/state` | Exact HTTP body bytes | Publisher mirrors the current boot, grants and target IDs |
| POST `/access` | Exact HTTP body bytes | Connector requests an authorized control or attach channel |
| POST `/channels/{channel_id}/release` | Exact `{}` body | Connector cancels its own channel before retrying |
| POST `/channels/{channel_id}/authenticated` | Exact `{boot_id}` JSON body | Publisher confirms completed inner authentication |
| POST `/status` | Exact `{}` body | Publisher-only channel pressure and configured quotas |
| GET `/channels/{channel_id}/{role}` | `X-Relay-Ticket` value | Consume device-bound publisher/connector role ticket |

No secret is placed in a URL. The redemption secret never enters the outer
protocol. Invitation routes are retained through the local receipt-recovery
interval, but a route never grants access: the publisher's authenticated TLS
redemption transaction is authoritative.

The Device DO persists a channel reservation and the two ticket digests before
notifying the publisher and returning the connector's ticket. Each ticket is
single-use, scoped to its device, role, channel and publisher boot. A repeated
role fails without replacing the existing socket. Both endpoints actively open
WSS; `route_ready` is sent only when both roles are present. The endpoints then
establish inner TLS and compare BIND contexts before sending the secret.
After redemption the publisher waits for an encrypted `{request_id}` receipt
acknowledgement before closing. The connector persists a successful grant
before acknowledging it; rejection is acknowledged too. A lost acknowledgement
does not invalidate a received result, and a lost response remains recoverable
with the persisted request ID.

Pair channels use the 32-byte DATA/CREDIT envelope from `README.md`, with exact
64-bit offsets. They have a 30-second total lifetime, at most 64 KiB of ciphertext
per direction, 16 KiB DATA payloads and 256 messages per incoming direction.
This finite-transcript transport is not used for native Herdr streaming. Pair and control channels
share a quota of four per publisher. Unknown frames,
offset violations, excessive data and wrong channel IDs end both roles.

The DO uses incoming Hibernation WebSockets, socket attachments and persisted
SQL reservations. Cleanup is scoped to a random reservation generation as
well as channel ID, so an old close cannot release a later reservation. On
publisher disconnect, its channels close; old ciphertext is never retained or
replayed. Expiry is checked on messages and requests even if an alarm is late.

On reconstruction, DeviceRelay validates persisted role consumption, attachments,
identity/generation, grants, target uniqueness, credit and rate budget together.
Missing or conflicting state closes with `STATE_CONFLICT`; no role or fresh
credit is recreated. The publisher-only status includes a per-construction
instance ID, persistent maintenance counters and ciphertext offsets/message
counts. Use the [manual acceptance checklist](../docs/manual-acceptance.md) to
record actual sleep/wake behavior and platform costs for each deployment.

API implementation references: [Cloudflare Hibernation WebSockets](https://developers.cloudflare.com/durable-objects/best-practices/websockets/),
[SQLite transactions](https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/),
[Workers Ed25519 support](https://developers.cloudflare.com/workers/runtime-apis/web-crypto/).

## Authorized targets and native channels

The publisher sends `POST /state` with its boot ID, grant device IDs/versions
and opaque target IDs. No target names, sessions or local paths are uploaded.
A new pairing receipt is returned only after the durable grant is mirrored.
A new publisher control attachment starts unready: access returns retryable
`HOST_OFFLINE` until the initial state transaction is committed, rather than
misreporting absent startup mirrors as authorization revocation.
The connector requests `POST /access` with a new channel ID, purpose `control`
or `attach`, and a null or opaque target ID respectively. The Device DO checks
its grant mirror; the host checks its current local grant again during inner
TLS and immediately before opening a native socket. A cloud mirror is never
authoritative for access to Herdr.

Every authorized channel records a positive safe-integer grant version from
the publisher's current mirror. The version travels in both role offers and
is compared inside the exact mutual-TLS BIND context. Pair channels have a
null grant version. Joining a role rechecks the recorded version against the
current mirror; the endpoint rechecks it before TLS and before native attach.

Revocation atomically removes the local incoming grant and advances a
persistent monotonic serial. A local notification cancels all operations for
that grant, including pending TLS handshakes, before cloud synchronization.
The Device DO closes stale reservations and rejects their unused tickets when
the new state arrives. Grants in the other direction and grants for other
devices are independent. Regranting uses a newer version; a receipt is only
recoverable while its exact original grant version remains active. Restart
publishes the persisted current state before the service reports online.

Control channels return target names and status only inside mutual TLS. An
attach gets its own role tickets, WSS pair and inner TLS connection, binding
the target and boot generation. The host reserves the actual native socket
inode before returning encrypted `ready {runtime_id}`. Only after the local
Herdr client connects to its private bridge does the connector send encrypted
`{runtime_id}`. The host revalidates the grant and socket identity, opens the
native socket and sends `started`. From that point onward, the TLS plaintext
is exclusively native bytes; errors close the connection instead of inserting
control messages. Framed control reads consume exact lengths and preserve
any following bytes in the same TLS read buffer.

A Room DO stores one atomic reservation for each `(publisher, target)`.
The current implementation keeps socket routing in DeviceRelay and the
exclusive lease in Room. Pending reservations expire in 30 seconds. Once both
roles join, the lease lasts 120 seconds and is renewed by a 30-second Device
alarm. Every data event checks the persisted local deadline before forwarding;
a failed or late renewal closes the old channel. Deletion/release matches a
random reservation generation, so late cleanup cannot free a newer occupant.
The host's inode lock independently rejects duplicate or aliased attaches.

Native DATA uses 16 KiB chunks and a 64 KiB sliding window in each direction.
All offsets remain exact u64 values (decimal strings in socket attachments).
The endpoint's extra duplex buffer is 16 KiB and its inbound queue is capped
at 64 messages as well as the byte window. Credit is returned after bytes are
written into the bounded endpoint stream. WSS writes time out after ten
seconds; native ping is sent every 25 seconds with a 75-second liveness limit.
Dropping a stream explicitly cancels its pump even when no sending credit
remains. No data is retained for a subsequent connection. Each publisher permits eight attach channels, four combined pair/control channels,
and four unauthenticated channels. Attach admission also reserves the target's
single slot. All categories include pending reservations. The publisher marks a
non-pair channel authenticated only after inner TLS, exact context binding and a
fresh local grant check; the request is signed for the current boot. Pair
channels remain pending until they close. Authentication releases the separate
handshake semaphore, never the attach/short-channel quota.

Self-hosted configuration can lower `MAX_DEVICE_ATTACH` (1–8),
`MAX_DEVICE_SHORT` (1–4), `MAX_PENDING_CHANNELS` (1–4), and
`CHANNEL_BYTES_PER_SECOND` (16384–262144). Invalid configuration fails closed.
The one-attach-per-instance constraint and 64 KiB window cannot be raised.
Admission stores the channel's rate and sends it in the access offer and open
response as `limits.bytes_per_second`. Quota denial is terminal for the current
connect command and never evicts another channel.

Each direction has a DATA token bucket: 256 KiB/s by default and a burst of
max(64 KiB, twice the configured rate). DATA and CREDIT together allow 150
messages/s with a burst of 300. Rust paces at 95% of the advertised rate to
allow ordinary batching and timer variation; the relay rejects excessive
traffic. Duplicate CREDIT consumes message budget but is not forwarded and
does not reset progress. CREDIT never exceeds the opposite role's sent offset.
Token balances, offsets and last forward progress persist in attachments.

Thirty seconds without advancing outstanding credit closes only that channel;
Pong, duplicate CREDIT and unrelated traffic do not extend it. Rust independently
bounds incoming-consumption and outgoing-credit stalls. Cloud alarms enforce the
same deadline even when the peer is silent. No queued bytes survive closure.
The signed publisher-only status route exposes ciphertext bytes outstanding,
peak window use, bounded close counters and limits. It exposes no terminal data,
display names, paths, tickets or secrets. Workers has no documented send-queue
length API; these metrics measure application pressure, not private platform
queue internals. See the [manual acceptance checklist](../docs/manual-acceptance.md)
for resource checks and the [Herdr compatibility patch](../patches/herdr/README.md)
for the separate upstream control-queue limit.

Target definitions now persist separately in private `targets.json` snapshots.
`share add/list/enable/disable/remove` manage exact session/socket locators and
stable IDs. The service reloads after each committed mutation and cancels only
channels whose target was disabled or removed. Reloading an unchanged target
preserves its runtime object. Cloud state synchronization is serialized and
uses the current target registry and grants; display names and locators stay
local. Disabled definitions survive service restarts.

## Recovery and cleanup

A connector first closes its old TLS/native streams and reaps the owned Herdr
client. It then sends a signed release for its previous channel, including when
an open response was lost. Only the original connector may release an existing
channel. An absent channel gets a two-minute cancellation tombstone so a delayed
open transaction cannot resurrect it; tombstones are bounded to 128 per device.
The release closes both role sockets, deletes the exact reservation and awaits
its Room release before acknowledging. Native inode locking remains authoritative
if the host is still closing its old socket. `TARGET_BUSY` always terminates
recovery, even for a second client using the same device identity.

Recovery creates a new random channel, fresh role proofs/tickets and a full TLS
handshake. There is no ciphertext resumption or native input replay. The CLI
consumes and discards offline input, accepts Ctrl-C cancellation, and flushes
pending terminal input before launching a new native client. Successful native
detach ends the foreground command. Authentication, identity, protocol, client
version, removed/disabled target and busy errors terminate recovery.

Transient failures use equal jitter: initially 0.5–1 seconds, doubling up to a
15–30 second interval. A server Retry-After, in seconds or HTTP date form, is a
minimum wait and can exceed that interval; waiting remains cancellable. A
connection lasting at least 60 seconds resets the backoff. Endpoint heartbeats
run every 25 seconds with an independent 75-second silence deadline; writes
have bounded deadlines.

The publisher keeps local IPC available while reconnecting. A fresh signed
publisher control connection ends the older boot's channels before replacing
that connection, including when it is half-open. Each reconnect uses a new boot
and mirrors the current persisted grants and enabled targets. It does not reopen
native sockets until a newly authenticated attach requests them.
