# Production acceptance — 2026-10-10

Production is `https://herdr-relay.hackerchai.com`, with separate SQLite Durable
Object namespaces, public accountless admission and the committed production
quota/retention settings. The operator profile is held outside the repository.
The initial domain exercise used commit `2d0b7c7` (bootstrap version `0.0.0`) and
the pinned custom Herdr `0.9.3-custom.relay-control-bound-1`; it does not claim to
be the subsequently versioned public crate.

The completed [0.1.1 release](2026-10-10-release-0.1.1.md) subsequently deployed
Worker version `2fe7c8da-d54d-4d83-b4fa-f0fa6fb8d725` at 100% traffic. The CLI was
installed directly from public crates.io into an isolated local directory, and
the native server/client came from the verified macOS arm64 GitHub release
archive. With fresh private profiles, that combination passed pairing, discovery,
Unicode native input, same-target busy rejection while preserving the holder,
manual reconnect, revocation in 2.430 seconds, rejected access after revocation
and fresh pairing. Terminal attributes were checked after each native exit;
Herdr survived teardown. These are actual release-artifact results on macOS 27
arm64, not a relabelling of the bootstrap run or new GUI testing.

Fresh private profiles completed invitation redemption, encrypted discovery,
native Unicode input, same-target `TARGET_BUSY` without disrupting the holder,
manual reconnect, revocation, denied discovery after revocation and fresh pairing.
Revocation terminated the active CLI in 2.533 seconds. All observed native exits
restored the terminal; the isolated Herdr server survived connection teardown.

A production configuration exercise deployed `ADMISSION_ENABLED=false`, observed
`503 RELAY_BUSY`, and used Wrangler rollback to restore the exact previous
Worker version. The TLS/protocol probe passed after rollback; DO namespaces and
stored counters were retained. A second fresh-profile run then verified native
operations, publisher-service restart with automatic client reattachment, no
replay of offline input, revocation (2.703 seconds), and fresh pairing. No private
profile or invitation was copied into the repository. Temporary services and
native servers were stopped; pre-existing user Herdr sessions were not touched.

[Sanitized measurements](2026-10-10-production-metrics.json) identify each Worker
version and the checks. Manual drivers/logs are private under
`target/acceptance/2026-10-10/`; no permanent e2e suite was added. GUI acceptance
is the maintainer's explicit confirmation, separate from these native PTY runs.

Workers Free is retained, including provider-enforced failure on free quota
exhaustion. The deployment preflight verified no paid Workers/DO subscription.
Account-wide budget alerts at USD 5, 8 and 10 are enabled for the maintainer's
Cloudflare email. Alerts include other account products and are not a hard
spending stop. The project's USD 10 policy relies on retaining Free, never on
assuming an alert instantly cuts off a paid service.

A finite post-session request exercise sent 33 malformed unsigned requests:
30 returned `400 PROTOCOL_ERROR` and three returned `429 RATE_LIMITED` with
`Retry-After: 60`. Prior normal session requests had already consumed part of
the shared-IP minute allowance. The real operator CLI then reported
`RATE_LIMITED` and exited 1. No paid plan or quota was raised for this exercise.
