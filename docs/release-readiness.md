# First release readiness

Updated 2026-10-10. The maintainer accepted GUI testing, chose MIT licensing,
kept GitHub private and authorized public crates.io publishing. The production
hostname is `herdr-relay.hackerchai.com`; the monthly project ceiling is USD 10.
[ADR-0013](adr/0013-private-repository-public-crate.md) records these changes.

| Gate | Current state | Evidence / remaining work |
| --- | --- | --- |
| CLI, encrypted trust, native transport and user services | Implemented and exercised | [Candidate records](validation/2026-10-10-candidates.md) |
| GUI terminal acceptance | Passed by maintainer confirmation | Direct instruction on 2026-10-10; no new GUI tool run claimed |
| Four candidate platforms | Prior candidates exercised | New release artifacts must be built from the CI release tag; Rosetta/VM limits remain documented |
| Upstream reliable-control queue | Patch implemented; 32 transport tests pass | [Patch and build instructions](../patches/herdr/README.md); custom server required for this bound |
| Rust/TypeScript checks | Local checks pass; CI configured | [Release workflow](releasing.md); actual hosted CI runs pending |
| License and publication | MIT; private GitHub, public crate | Crate explicitly includes Worker source and patch materials |
| Production Worker | Deployed; native acceptance in progress | Custom hostname and independent SQLite DO namespaces |
| Budget | Retain Workers Free, below USD 10 | Provider rejects paid-only CPU setting with error 100328; CI rejects a paid Workers/DO subscription |
| Retention | Existing defaults retained | 25-hour offline mirrors; [other TTLs](operations.md); no application payload logs |
| Availability and service scope | Best effort; user services | No SLA, forced logout test or system service promise |
| Public crate and release handoff | Pending | Fresh production pairing, rollback, all CI checks and actual registry publication remain required |

Cloudflare resources remain in the maintainer's existing account. The operator
identity is held in a private local profile; only its public device ID is in
production configuration. Repository Secrets hold the crates token and a scoped
Cloudflare token, not the Global API Key. Do not publish private profiles,
credentials, invitations or raw diagnostic captures.

This file records actual state; workflow source existing is not evidence that a
release ran successfully. The old `0.1.0-dev` artifacts and their checksums remain
historical candidates and are not relabelled as the new release.
