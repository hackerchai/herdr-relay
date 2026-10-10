# CI and release operation

Every push, pull request and merge queue commit runs `CI Required`. That gate
requires Rust formatting, Clippy with warnings denied, Rust module tests and
crate packaging on Linux/macOS; TypeScript formatting, ESLint, type checking,
module tests and Worker bundling; and the pinned Herdr queue regression tests.
No permanent e2e suite is installed. Overall acceptance remains manual.

Conventional commits feed Release Please after a successful main-branch CI run.
Its PR updates Cargo manifests/lockfile, the Worker package/lockfile, changelog
and release manifest together. Because GitHub suppresses ordinary events created
with `GITHUB_TOKEN`, the release workflow explicitly dispatches CI on the bot PR
branch. The same checks therefore also run on release PRs.

Merging a version PR creates a `vX.Y.Z` release and invokes `Publish` in the same
workflow. Publication validates the tag, main ancestry and synchronized versions,
reruns checks on that exact commit, and builds native packages for macOS arm64,
macOS x86_64, Linux arm64 musl and Linux x86_64 musl. Each also builds the pinned
bounded Herdr server. Checksums and source manifests accompany the private GitHub
assets. The CLI source is published publicly to crates.io under MIT; Worker
source and the upstream patch/build materials are explicitly included.

After artifact construction succeeds, the workflow verifies the account remains
on Workers Free and deploys the matching Worker through pinned Wrangler. A TLS
and protocol probe checks the production hostname. Every stage must pass;
an incomplete run is not a successful release. Retry with the `Publish` workflow's
existing `tag` input. An existing crate is accepted only if its checksum exactly
matches; immutable registry versions with different contents require a new release.

Repository Secrets:

| Name | Use |
| --- | --- |
| `CARGO_REGISTRY_TOKEN` | Publish `herdr-relay` to crates.io |
| `CLOUDFLARE_API_TOKEN` | Scoped Workers deploy, zone routes and billing read |
| `CLOUDFLARE_ACCOUNT_ID` | Deployment account |

The Cloudflare token has no billing-write permission. It is scoped to the existing
account and `hackerchai.com` zone; the Global API Key is not placed in CI.
Keep repository visibility private. Publication uses repository Secrets and adds
no manual approval step. GitHub rejected branch protection for this private
repository on its current plan (403: GitHub Pro required). `CI Required` reports
all checks, and Publish enforces their success, but GitHub cannot enforce the
ordinary PR merge gate on this plan. Do not change visibility or buy a plan
automatically.

## Recovery

Record the previous Worker version before deployment. Use `wrangler rollback
VERSION --config wrangler.production.jsonc` to restore Worker code with unchanged
DO bindings. Rollback is not a database migration reversal; these initial v1–v3
migrations only introduce SQLite DO classes. Never remove production namespaces
to roll back code. Recheck pairing, discovery and an existing native session.

For emergency closure, deploy `ADMISSION_ENABLED=false` on the existing Worker.
New admission stops and existing leases close without killing Herdr sessions.
Restore only after checking the cause. An already published crate cannot be
overwritten; ship a patch version (or explicitly yank a defective version after
review). Local private profiles remain authoritative and should be backed up
without copying them into this repository.
