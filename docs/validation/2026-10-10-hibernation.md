# Cloudflare hibernation and measured overhead — 2026-10-10

Issue: [#12](https://github.com/hackerchai/herdr-relay/issues/12).
Controlled live deployment: `herdr-relay-development.hackerchai.workers.dev`,
version `41c07493-f7e4-4845-a3eb-2bb8850b9083`, restricted to three test identities.
Both endpoint machines were isolated profiles on macOS arm64 with Herdr 0.9.3.
This validates the real Cloudflare runtime, not four-platform release readiness.

The later v3 global-admission coordinator adds publisher/channel lease RPCs and
expiry alarms. The figures below describe the named earlier version and must not
be reused as the v3 cost estimate; see the admission validation and operations guide.

## Observed sleep, wake and routing

On 2026-10-09 from 21:57:20 to 22:00:41 UTC, two native clients attached to
separate Work and Build targets on one publisher. There was no deployment or
publisher restart during the interval. Both used native Ping every 25 seconds.

| Observation | DO instance prefix | Persistent construction count | Active attaches |
|---|---|---:|---:|
| Initial | `2ab0a1d1` | 2 | 2 |
| After 45 seconds idle | `c2caa0c4` | 4 | 2 |
| After a second 45-second idle | `76ebd9be` | 6 | 2 |
| Build detached, then 45 seconds idle | `6c4d848d` | 8 | 1 |
| Build attached afresh, then 45 seconds idle | `859f2074` | 10 | 2 |
| Incoming device grant revoked | Same as preceding row | 10 | 0 |

The first idle interval preserved both channel IDs and every sent/ack offset and
application-message count exactly. New markers after wake reached the respective
Work and Build panes. The second idle interval preserved the progressed offsets
and IDs. Neither client reconnected. Work stayed attached for longer than the
120-second Room lease, demonstrating successful renewal across reconstruction.
A second attach to occupied Work after hibernation returned `TARGET_BUSY`.

Build's closed role did not reappear on wake. Its slot admitted a fresh channel
with a different ID; Work remained on the original channel. After the last idle
period, local revocation advanced the incoming grant to version 9 and both
clients ended with status 1. Signed publisher status contained no channels.
Both terminals were restored, and Herdr servers remained running. A later grant
was explicitly reissued for further controlled development tests.

An unused connector role ticket was also retained through 35 seconds of idle.
Its later upgrade was rejected with `403 TICKET_INVALID`. A fresh reservation
succeeded, and repeated release remained idempotent. No ticket was logged.

## Recovery rules

`acceptWebSocket` registers inbound Hibernation WebSockets. Per-role attachment
state contains exact offsets, credit, token balances, progress timestamps,
message counts, purpose, identity and reservation generation. SQL separately
holds ticket-consumption bits, grant version, quotas and lease deadlines.

The constructor validates these authorities together. Missing or duplicate
consumed roles, wrong identities/generations, missing flow state, impossible
credit, expired channels, changed grants and duplicate target occupancy close
conservatively. A consumed role is never recreated from SQL and credit is never
reset on wake. Local native inode locks remain an independent exclusivity check.
A focused unit test covers preserved nonzero credit and these recovery conflicts.
Missing/corrupt attachment injection is a unit-level check, not a claimed live
Cloudflare corruption experiment.

Thirty-second alarms renew active Room leases to 120 seconds and enforce slow
consumer deadlines. Native Ping is handled by the platform without application
messages: in the first 45-second interval, role message counts remained exactly
unchanged while construction and alarm counts advanced. Maintenance does cause
wakeups and has a measurable storage/RPC cost; idle connections are not described
as free. Per-role data and credit continue from their persisted values.

## Platform metrics

Cloudflare GraphQL was queried for the exact 201-second interval above. Values
below are returned platform aggregates, rather than extrapolated monthly costs.
The query selected only this development Device namespace and its two active
Room objects. Duration is GB-seconds; active and CPU time were returned in
microseconds and converted to seconds.

| Object | Active s | CPU s | Duration GB-s | Rows read | Rows written | Outgoing WS messages |
|---|---:|---:|---:|---:|---:|---:|
| Publisher Device | 1.006002 | 0.099431 | 0.128768256 | 555 | 42 | 58 |
| Build Room | 0.327879 | 0.007414 | 0.041968512 | 24 | 7 | 0 |
| Work Room | 0.184797 | 0.002869 | 0.023654016 | 20 | 5 | 0 |

The Device reached five WebSockets (one control and four role sockets).
Its reported maximum isolate memory was 2,565,809 bytes; Room samples reported
1,572,876 bytes. These measurements are shared-isolate memory, not object-private
allocations or network send-queue sizes.

Invocation aggregates reported 57 hibernation message events, 6 Device alarms,
7 Device HTTP events, 13 Room RPCs, 26 AccessGate RPCs and 5 AccessGate alarms,
with no errors in this interval. Periodic incoming-message count was zero because
hibernation messages are accounted in the invocation dataset; that zero does not
mean there was no inbound traffic. The cost table excludes AccessGate and Worker
compute, storage retention, network/account-wide overhead and pricing tiers.
These exclusions must be included in any operational budget. Analytics can be
sampled and delayed; these figures are an observation, not an invoice prediction.

The publisher's application counters advanced by six alarms and eleven renewals
between the initial and pre-revocation snapshots. The latter includes the fresh
Build attach's renewal; namespace RPC aggregates include reserve/release work.
Do not equate RPC count with every SQL operation or with WS frame count.

Sources: [Cloudflare metrics](https://developers.cloudflare.com/durable-objects/observability/metrics-and-analytics/),
[Hibernation API](https://developers.cloudflare.com/durable-objects/best-practices/websockets/),
[Alarms](https://developers.cloudflare.com/durable-objects/api/alarms/), and live
GraphQL schema descriptions for activeTime, cpuTime and duration.
