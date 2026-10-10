# ADR-0013: Private repository, MIT source distribution and budget boundary

Accepted 2026-10-10. The maintainer selected a private GitHub repository, MIT
licensing and public crates.io distribution. This supersedes the public GitHub
publication assumption in ADR-0002 and ticket #20: the crate includes CLI and
Worker source, self-host materials and the separately Apache-2.0 licensed Herdr
queue patch; GitHub release assets remain private. The default Relay hostname is
`herdr-relay.hackerchai.com`, with a maximum monthly project budget of USD 10.

The current Cloudflare account is on Workers Free, confirmed by subscriptions
and the provider rejecting configurable CPU limits with Free-plan error 100328.
Keep Workers/SQLite DO on that plan, where exhaustion fails requests instead of
buying overages; the deployment preflight rejects a Workers/DO subscription.
Do not upgrade billing automatically. Other account products and future plan
changes are outside this project's cost guarantee.

The maintainer has accepted GUI testing as passed. Preserve that human acceptance
separately from the earlier protocol captures; no new GUI tool run is claimed.
Keep the existing user-service scope and metadata retention defaults. This first
personal service is best effort and does not add an SLA or system-level service.
