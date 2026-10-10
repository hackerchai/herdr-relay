# First release readiness

Updated 2026-10-10. The maintainer accepted GUI testing, chose MIT licensing,
kept GitHub private and authorized public crates.io publishing. The production
hostname is `herdr-relay.hackerchai.com`; the monthly project ceiling is USD 10.
[ADR-0013](adr/0013-private-repository-public-crate.md) records these changes.

| Gate | Current state | Evidence / remaining work |
| --- | --- | --- |
| CLI, encrypted trust, native transport and user services | Implemented and exercised | [Candidate records](validation/2026-10-10-candidates.md) |
| GUI terminal acceptance | Passed by maintainer confirmation | Direct instruction on 2026-10-10; no new GUI tool run claimed |
| Four release platforms | Tagged native builds and executable checks passed | [0.1.1 release record](validation/2026-10-10-release-0.1.1.md); prior interactive Rosetta/VM limits remain documented |
| Upstream reliable-control queue | Patch implemented; 32 transport tests pass | [Patch and build instructions](../patches/herdr/README.md); custom server required for this bound |
| Rust/TypeScript and upstream checks | Release CI passed on Linux/macOS | [Successful release run](https://github.com/hackerchai/herdr-relay/actions/runs/38028537984); 40 Rust tests per OS, 7 TypeScript tests and 32 upstream transport regressions |
| License and publication | MIT; private GitHub, public crate | Crate explicitly includes Worker source and patch materials |
| Production Worker | 0.1.1 deployed; registry-installed CLI acceptance passed | [Production checks and rollback](validation/2026-10-10-production.md); released CLI revocation took 2.430 seconds |
| Budget | Retain Workers Free, below USD 10 | Provider rejects paid-only CPU setting with error 100328; CI rejects a paid Workers/DO subscription |
| Retention | Existing defaults retained | 25-hour offline mirrors; [other TTLs](operations.md); no application payload logs |
| Availability and service scope | Best effort; user services | No SLA, forced logout test or system service promise |
| Public crate and release handoff | Published and verified | [Public 0.1.1 crate](https://crates.io/crates/herdr-relay/0.1.1), [private binary/source assets](https://github.com/hackerchai/herdr-relay/releases/tag/v0.1.1), [checksums and handoff](validation/2026-10-10-release-0.1.1.md) |

Cloudflare resources remain in the maintainer's existing account. The operator
identity is held in a private local profile; only its public device ID is in
production configuration. Repository Secrets hold the crates token and a scoped
Cloudflare token, not the Global API Key. Do not publish private profiles,
credentials, invitations or raw diagnostic captures.

This file records actual state; workflow source existing is not evidence that a
release ran successfully. The old `0.1.0-dev` artifacts and their checksums remain
historical candidates and are not relabelled as the new release.

GitHub rejected private-repository branch protection on the current Free plan.
`CI Required` reports all checks and publication enforces them; ordinary PR
merges cannot be made subject to a server-enforced required check on this plan.
The repository remains private. See [release operation](releasing.md).
