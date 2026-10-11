# Controlled Relay operations

Production uses `herdr-relay.hackerchai.com` in the maintainer's Cloudflare account.
The project uses a public source repository, MIT/public crate distribution, a USD 10
monthly ceiling and best-effort user-service scope. Production overrides are in
`relay/wrangler.production.jsonc`; the table
below gives general self-host defaults.

The account runs Workers Paid (USD 5 base). The [monthly budget](#monthly-budget)
nearly stops the relay before usage exceeds what that plan includes, so the
deployment does not buy overages. Deployment reports the plan and refuses to
deploy the public relay with an access token. Account budget alerts at USD 5, 8
and 10 are enabled for `dns_manage@hackerchai.com`. These alerts include other
account products and do not themselves stop spending.
Application payload logging remains disabled and offline mirror retention is 25h.

## Monthly budget

`Admission` meters each UTC month's usage the way Workers Paid bills it and
refuses new work once any allowance is spent. The defaults are 90% of what the
plan includes, leaving headroom for work the meter cannot see (CPU time,
WebSocket pings, isolate-cached refusals):

| Var | Default | Workers Paid includes |
| --- | ---: | ---: |
| MONTHLY_WORKER_REQUESTS | 9000000 | 10 million requests |
| MONTHLY_DO_REQUESTS | 900000 | 1 million Durable Object requests |
| MONTHLY_DO_GB_SECONDS | 360000 | 400,000 GB-s |
| MONTHLY_DO_ROWS_WRITTEN | 45000000 | 50 million SQLite rows written |

`MONTHLY_BUDGET` is `on` (default) or `off`. Each value is a positive integer and,
unlike the engineering quotas below, may be raised by an operator who accepts
overage charges.

The meter is an estimate that errs high. Every Durable Object invocation (RPC
session, fetch or alarm) counts as one request, 10 ms of 128 MB duration and 4
rows written. Incoming WebSocket messages count 1/20 request and 1 ms each, as
billed. The Worker reports the calls each entry request made; DeviceRelay
reports its frames, alarms and outgoing calls with the Admission calls it
already makes (channel renewals every 30 seconds, releases and publisher
renewals). Frame counts live in hibernation-safe socket attachments, so waking
from hibernation loses none.

Once an allowance is spent, entry requests, new leases and lease renewals return
`GLOBAL_QUOTA_EXCEEDED` with `Retry-After` up to one hour (never past the start
of the next month). Each Worker isolate then refuses requests itself for a
minute before asking `Admission` again, so a stopped relay costs Worker
requests but almost no Durable Object work. Active channels close at their next
renewal (about 30 seconds) and publishers when their lease expires (at most ten
minutes). Service resumes automatically in the next UTC month, or within an
hour of raising a limit. `relay status` shows the month's usage and limits; compare
them with the account's billing analytics.

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

The per-device, per-target, rate and credit-window constraints still apply.
Creating new identities cannot enlarge any global pool. An IP bucket is only a
short rate gate: users behind a NAT share it, so widespread `RATE_LIMITED` must
be investigated before lowering it. No registration or payment is introduced, and
the public relay never requires an access token (self-hosted relays may). Host presence has no daily uptime allowance or maximum
connection age: a healthy host renews its bounded concurrency slot indefinitely.
`GLOBAL_PUBLISHER_SECONDS_PER_DAY` is retired and ignored, including in older
self-host configurations. The `publisher_ms` counter remains observational.

`429` and `503` include `Retry-After`. Publishers and reconnecting clients wait
through temporary global quota exhaustion, with jitter after the server's
cooldown to avoid synchronized retries. Without a cooldown, global quota and
rate-limit failures wait at least 60 seconds. Local service commands remain
responsive during the wait. Invalid authorization, changed identity and TLS
authentication failures remain terminal; retries never bypass these checks.
Challenge signing tolerates up to five minutes of endpoint clock skew without
changing the Relay's nonce expiry. A larger offset reports `CLOCK_SKEW` and
retries no faster than once per minute; enable automatic time synchronization.

Request leases last 15 seconds; body reads expire after 10 seconds with `408
REQUEST_TIMEOUT`. Challenges expire after 60 seconds, pending channels after 30.
Publishers have ten-minute leases, normally renewed every five minutes. A
transient renewal failure retries after 30 seconds (or a longer server cooldown)
while retaining only the already granted lease. The retry schedule survives
hibernation. A missing/expired lease still closes the connection and the daemon
reconnects automatically. Active attach channels retain 120-second leases and
30-second maintenance alarms. Exact owner/reservation matching makes delayed
release harmless. A missing/expired global lease fails closed, including after
upgrade from a version without the global coordinator.

Data-channel service time is **prepaid**, not a measurement of observed CPU or billed duration.
Each lease extension charges only the newly reserved interval, split across UTC
midnight. Early release does not refund it; rapid reconnects cannot obtain free
service time. Reservations spanning two days debit neither day if either quota
would be exceeded. This intentionally conservative cap may reject idle clients.
It bounds admitted service time, alongside the per-direction byte/message rates;
it is not an exact egress allowance or dollar spending limit. Counters persist
through hibernation, process restart and configuration updates. Reaching the
channel budget can interrupt an attach, but does not evict the host publisher.
Requests and data channels still obey application and provider quotas. Workers
Free exhaustion, endpoint sleep and provider outages can interrupt access; this
design provides automatic recovery, not a zero-downtime guarantee.

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
Idle publishers use one global renewal RPC every five minutes; active channels
renew roughly every 30 seconds. WebSocket protocol ping/pong frames do not wake
hibernating objects. Entry/challenge operations and expiry alarms add work.
Measure the current deployment; application uptime counters are not billing data.

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
| Global leases | Requests 15 seconds; channels at most 120 seconds; publishers at most ten minutes, with exact-owner release or expiry |
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
