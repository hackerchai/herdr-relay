# Global admission acceptance — 2026-10-10

Issue: [#17](https://github.com/hackerchai/herdr-relay/issues/17).
Implementation: SQLite Admission DO, stable `global-admission-v1` name and v3
migration; entry/challenge/publisher/channel leases, prepaid UTC daily service
time, operator-signed aggregate status and finite metadata cleanup. No e2e suite
was added. Two focused SQLite unit tests cover shared quota reconstruction,
expiry, real transaction rollback, cross-midnight charging, exact-owner release
and replacement publisher ownership.

## Local workerd controls

Wrangler 4.149.0 / workerd 1.20261006.1, persistent isolated state, ordinary Rust
CLI and official Herdr 0.9.3. Tests lowered separate limits so the asserted
rejection was not hidden by a different quota. Native Work/Build instances and
three new private device profiles were dedicated to acceptance.

| Scenario | Observed result |
| --- | --- |
| Eight fresh Ed25519 identities, global minute limit 6 | First six challenges succeed; seventh/eighth `429 RATE_LIMITED`, Retry-After 60 |
| Config reload after spending the minute quota | Still `429`; durable counters did not reset |
| Two incomplete HTTP bodies, pending limit 2 | Third request `503 RELAY_BUSY`, Retry-After 15 |
| Slow-body expiry | Both return `408 REQUEST_TIMEOUT` after the 10-second deadline; capacity recovers |
| Challenge pool limit 3, different new identities | Fourth request `429 GLOBAL_QUOTA_EXCEEDED`; rows expire and normal pairing resumes |
| Publisher limit 1 | A second publisher exits with `GLOBAL_QUOTA_EXCEEDED`; first remains online |
| Pending-channel limit 1, total-channel limit 2 | Second distinct target rejected globally before authentication; explicit release succeeds |
| Active-channel limit 1 | Second distinct target rejected; original native client continues operating |
| Release then attach another target | Succeeds, no leaked Room/global reservation; native exit 0 and restored termios |
| Operator status | Authorized device obtains aggregate counters; ordinary device is rejected |
| Admission off | Active client disconnects in 0.27 s during local config reload; signed operator status reports off |
| Offline cancellation after admission off | Exit 130, original termios restored, native Herdr workspace still running |
| Restore admission and publisher | Existing local pairing and all three saved targets become available again |

The local reload timing is not a guaranteed cloud kill-switch latency. Runtime
handlers reject forwarding when disabled and maintenance renewals fail closed;
the nominal maintenance interval is 30 seconds with a 120-second lease bound.
Quota rejection can retain conservative leased capacity until exact release or
expiry. Budget alerts and application admission limits are distinct controls.

The slow-body test initially returned 500 because cancellation rejected the
reader before its normal completion path. The deadline now maps that rejection
to 408; rerunning the same two slow requests confirmed both the status and slot
recovery.

Offline metadata cleanup was exercised using the local Wrangler Explorer SQLite
API to advance only the isolated device's expiry deadline, avoiding a 25-hour
wall-clock wait. Before cleanup its cloud mirror held one grant, three targets
and one invitation route. After stopping its publisher and invoking the expired
object, those rows, counters and expiry metadata were all zero. Identity/target
file hashes on the endpoint were unchanged. Restarting the publisher restored
the existing authorization and all three targets from local state. The expiry
injection is a manual test technique, not an exposed production endpoint.

## Real Cloudflare path

The controlled self-host Worker added its own fourth SQLite namespace. Version
`45d361cb-1aad-4faf-96e9-fd313cf758de` accepted the new global admission path.
Only two test devices were allowed, and the operator allowlist contained the
publisher's public device ID. The private key remained in its local profile.

The ordinary native client received its complete image screen before input,
then passed the [terminal payload matrix](2026-10-10-terminal.md). A further run
stayed idle for 135 seconds, crossing the original 120-second lease, and then
passed paste, mouse, resize, image and clipboard payload checks. It detached
with exit 0 and restored termios. Operator aggregate status succeeded; the
other signed device received `NOT_AUTHORIZED`.

The final candidate Worker source was subsequently deployed as
`61f4cb6f-e29f-4f80-99cb-653f4a01add8` on the self-host check and
`40b9997a-f39f-4101-85c6-6a0404831bc8` on the development check. These include
the metadata-expiry and diagnostic refinements; the 135-second idle sample
above belongs to the preceding v3 version, not these deployment IDs.

For that idle sample (2026-10-09 23:12:22–23:14:37 UTC), Cloudflare reported
DeviceRelay active time 1.123715 s, CPU 0.041011 s, duration 0.14383552 GB-s,
255 rows read / 23 written and 51 outbound WebSocket messages. Room reported
0.174919 s active, 0.003563 s CPU, 0.022389632 GB-s and 20 / 5 rows. The
invocation sample contained 18 hibernation events, six Device alarms, six Room
RPCs and two AuthGate alarms, with no reported errors. The maximum reported
Device isolate memory was 2,346,524 bytes; this is not object-private RSS.

Admission rows were initially missing, including the first delayed refresh.
A later refresh recovered them: 0.645974 s active, 0.010350 s CPU,
0.082684672 GB-s, 218 rows read / 34 written, 12 RPCs and three alarms, with no
reported errors. Maximum reported isolate memory was 1,572,876 bytes. This is
direct evidence of additional coordinator work; the earlier absence was not zero
cost. The three reported namespaces total 0.248909824 GB-s over the 135-second
window. AuthGate periodic rows and account-wide Worker costs are not included
in that subtotal. This sampled window cannot establish a complete hourly bill
or a production budget; preserve provider reporting latency in cost analysis.
The [sanitized metric rows](2026-10-10-admission-metrics.json) retain units,
per-class counters and invocation types without device/object identifiers.

These are controlled engineering results. They do not establish public abuse
resistance against arbitrary Internet floods, a dollar spending cap, four-platform
release acceptance or a GUI terminal matrix. The production operator, budget,
alerts and final retention policy remain #19 decisions. See
[operations](../operations.md) for enforcement scope and monitoring thresholds.
