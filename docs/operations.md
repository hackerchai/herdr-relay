# Controlled Relay operations

Production uses `herdr-relay.hackerchai.com` in the maintainer's Cloudflare account.
[ADR-0013](adr/0013-private-repository-public-crate.md) fixes the private repository,
MIT/public crate distribution, USD 10 monthly ceiling and best-effort user-service
scope. Production overrides are in `relay/wrangler.production.jsonc`; the table
below gives general self-host defaults.

Workers/SQLite DO remain on Free: exhaustion rejects requests rather than buying
paid overages. Deployment checks reject a Workers/DO paid subscription. Account
budget alerts at USD 5, 8 and 10 are enabled for `dns_manage@hackerchai.com`.
These alerts include other account products and do not themselves stop spending.
Application payload logging remains disabled and offline mirror retention is 25h.

## Global coordination

All Worker requests first call the single SQLite-backed `Admission` Durable
Object named `global-admission-v1`. The name is independent of device, IP, date,
origin and code version. Deployments retain its namespace and v3 migration;
do not rename/delete it to reset a quota. SQL transactions enforce the following
engineering defaults; string vars can lower each value to a positive integer.

| Var | Default | Scope |
| --- | ---: | --- |
| GLOBAL_REQUESTS_PER_MINUTE | 600 | Accepted entry requests across all identities and IPs |
| GLOBAL_REQUESTS_PER_DAY | 100000 | Accepted requests per UTC day |
| GLOBAL_IP_REQUESTS_PER_MINUTE | 240 | Shared public egress IP; not an account/device ban |
| GLOBAL_PENDING_REQUESTS | 32 | Request leases acquired before reading an unsigned body |
| GLOBAL_CHALLENGES | 256 | Unconsumed challenge reservations, at most 60 seconds |
| GLOBAL_PUBLISHERS | 64 | Published device connections |
| GLOBAL_CHANNELS | 32 | All attach, pairing and short control channels combined |
| GLOBAL_PENDING_CHANNELS | 64 | Channels not yet confirmed authenticated by the publisher |
| GLOBAL_CHANNEL_SECONDS_PER_DAY | 345600 | Prepaid channel time, all channels combined |
| GLOBAL_PUBLISHER_SECONDS_PER_DAY | 1382400 | Prepaid publisher time, all publishers combined |

The per-device, per-target, rate and credit-window constraints still apply.
Creating new identities cannot enlarge any global pool. An IP bucket is only a
short rate gate: users behind a NAT share it, so widespread `RATE_LIMITED` must
be investigated before lowering it. No registration, payment or common bearer
secret is introduced. `429` and `503` include `Retry-After`; the CLI treats global
quota exhaustion as terminal for the current operation and prints an actionable
message. Release spare connections or retry after the indicated interval.

Request leases last 15 seconds; body reads expire after 10 seconds with `408
REQUEST_TIMEOUT`. Challenges expire after 60 seconds, pending channels after 30.
Publishers and active attach channels have at most 120-second leases renewed by
30-second maintenance alarms. Exact owner/reservation matching makes delayed
release harmless. A missing/expired global lease fails closed, including after
upgrade from a version without the global coordinator.

Service time is **prepaid**, not a measurement of observed CPU or billed duration.
Each lease extension charges only the newly reserved interval, split across UTC
midnight. Early release does not refund it; rapid reconnects cannot obtain free
service time. Reservations spanning two days debit neither day if either quota
would be exceeded. This intentionally conservative cap may reject idle clients.
It bounds admitted service time, alongside the per-direction byte/message rates;
it is not an exact egress allowance or dollar spending limit. Counters persist
through hibernation, process restart and configuration updates.

## Inspect and alert

Set `OPERATOR_DEVICE_IDS` to a comma-separated list of trusted `identity show`
outputs. These public IDs authorize only aggregate operational reads. Keep their
private keys in normal private CLI profiles; never put them in Worker vars.

```sh
herdr-relay --profile /private/operator-profile relay status \
  --origin https://YOUR-RELAY
```

The signed response contains quotas, active publisher/channel/challenge/request
counts, pending channels and up to three UTC days of admitted requests,
rejections, HTTP error classes, opened leases and prepaid milliseconds. It
contains no device IDs, IPs, channel IDs, paths, tickets or terminal data. Entry
limits also cover this endpoint; use account analytics if entry is saturated.
Publisher-signed device status additionally reports in-flight credit, peak credit,
role byte/message counters, alarms, room renewals and failure-close counters.

For a controlled rollout, configure monitoring to poll aggregate status at most
once per minute and evaluate these initial thresholds. No monitoring schedule or
external notification is installed by the code:

| Signal | Initial engineering threshold | Action |
| --- | --- | --- |
| Global capacity or daily prepaid time | 80% warning; 95% critical | Inspect occupancy, shed new admission if needed |
| Global quota rejects | Any sustained increase for 5 minutes | Check abuse and legitimate demand |
| HTTP server errors | >1% over 5 minutes with at least 100 requests | Inspect platform errors and bindings |
| Active connections / slow-consumer closes | Unexpected growth or repeated closes | Inspect endpoint flow and native resource limits |
| Account cost estimate | 50%, 80%, 100% of the #19 monthly budget | Notify the #19 operator and follow its overage policy |

Use Cloudflare account Analytics/GraphQL for actual Worker requests/CPU/errors,
DO active time and GB-seconds, WebSocket counts, SQL rows read/written and storage.
Compare these to the prepaid/application counters; they measure different things.
The [hibernation measurement](validation/2026-10-10-hibernation.md) supplies a
baseline, but v3 adds one global renewal RPC per publisher/active channel roughly
every 30 seconds, plus entry/challenge operations and expiry alarms. Its original
cost samples must not be presented as v3 measurements.

Configure Cloudflare budget/usage notifications with the account's supported
notification features after #19 specifies recipients and thresholds. Budget
notifications do not enforce an immediate spending stop. Even a rejected request
incurs a Worker invocation and usually a coordinator call. Internet-scale floods
require Cloudflare edge DDoS/WAF/rate controls before this application; application
quotas cannot cap the provider's cost of receiving arbitrary traffic.

## Retention and shutdown

No application logging includes request bodies, proof/ticket headers, invitation
secrets or terminal bytes. Observability is disabled by default; enabling provider
logs requires reviewing its field capture and retention separately.

| Metadata | Retention / cleanup |
| --- | --- |
| Hashed IP minute buckets | Current and previous minute, then SQL/alarm cleanup |
| Global aggregate counters | Current and two preceding UTC days; no identities |
| Global leases | 15–120 seconds after last allocation, exact-owner release or expiry |
| Access nonces | 60 seconds maximum, consumed atomically once |
| Channel tickets and reservations | Pending 30 seconds; active until release/lease expiry |
| Late-release tombstones | 120 seconds |
| Invitation route hashes | Invitation expiry plus 24-hour lost-response recovery |
| Offline device grant/target mirrors and counters | `METADATA_RETENTION_HOURS`, default 25 hours, allowed 1–168 |

Offline mirror cleanup also deletes remaining invitation routes and tombstones.
Lowering retention below the invitation recovery interval intentionally shortens
cloud recovery after the publisher disconnects. Local identities, authorizations
and shared targets remain authoritative and are never deleted by cloud cleanup.
SQL row cleanup does not delete a Cloudflare namespace or its provider-managed
backups/PITR history; configure provider retention separately for production.

To stop admission, set `ADMISSION_ENABLED` to `false` and deploy the same Worker
name/bindings. New ordinary work returns `RELAY_BUSY`; existing data handlers
reject forwarding and maintenance stops lease renewal, closing both channel roles
and publisher control. No native Herdr server or agent is killed. Operator
challenge/status is the sole authenticated diagnostic exception, still bounded
by entry and challenge quotas. Restore `true` after resolving the incident;
clients reauthenticate and create new channels without replaying offline input.
For an account-wide emergency, disable the public route/Worker through Cloudflare
and inspect account-level resources; do not assume the app switch stops all bills.

References: Cloudflare [SQL transactions](https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/),
[alarms](https://developers.cloudflare.com/durable-objects/api/alarms/),
[analytics](https://developers.cloudflare.com/durable-objects/observability/metrics-and-analytics/).
