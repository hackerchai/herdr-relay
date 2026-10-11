# CI and release operation

Every push, pull request and merge queue commit runs `CI Required`. That gate
requires Rust formatting, Clippy with warnings denied, Rust module tests and
crate packaging on Linux/macOS; TypeScript formatting, ESLint, type checking,
module tests and Worker bundling; and the pinned Herdr queue regression tests.
No permanent e2e suite is installed. Overall acceptance remains manual.

Conventional commits feed Release Please after a successful main-branch CI run.
Feature commits increment the patch version while the project is pre-1.0,
keeping compatible releases in the current 0.2.x series.
Its PR updates Cargo manifests/lockfile, the Worker package/lockfile, changelog
and release manifest together. Because GitHub suppresses ordinary events created
with `GITHUB_TOKEN`, the release workflow explicitly dispatches CI on the bot PR
branch. The same checks therefore also run on release PRs.

Merging a version PR creates a `vX.Y.Z` tag and a draft release, then invokes
`Publish` in the same workflow. Tag creation is explicit so drafts can be checked
out before publication. Publication validates the tag, main ancestry and synchronized versions,
reruns checks on that exact commit, and builds native packages for macOS arm64,
macOS x86_64, Linux arm64 musl and Linux x86_64 musl. Each also builds the pinned
bounded Herdr server. Checksums and source manifests accompany the source repository's GitHub
assets. The CLI source is published publicly to crates.io under MIT; Worker
source and the upstream patch/build materials are explicitly included.

Native build caches cover compiler-output directories only. Verified upstream
source is prepared separately; restoring a pruned parent `target` cache can
otherwise recreate an incomplete source directory without its patch stamp.
The cache prefix was changed after this was observed during the first release.

After artifact construction succeeds, the workflow reports the account's Workers
plan (Workers Paid; spending beyond its base is bounded by the production quotas)
and deploys the matching Worker through pinned Wrangler. A TLS
and protocol probe checks the production hostname. Every stage must pass;
an incomplete run is not a successful release. All source-release archives and
the readiness manifest are uploaded before the draft is published and GitHub
locks its assets. Retry with the `Publish` workflow's existing `tag` input. If the
source release is already published, the retry downloads and verifies its
original assets instead of replacing them. A release accidentally locked before
the readiness manifest was uploaded requires a new patch version.
An existing crate is accepted only if its checksum exactly
matches; immutable registry versions with different contents require a new release.

Repository Secrets:

| Name | Use |
| --- | --- |
| `CARGO_REGISTRY_TOKEN` | Publish `herdr-relay` to crates.io |
| `CLOUDFLARE_API_TOKEN` | Scoped Workers deploy, zone routes, billing read and Workers R2 Storage edit (download site) |
| `CLOUDFLARE_ACCOUNT_ID` | Deployment account |

The Cloudflare token has no billing-write permission. It is scoped to the existing
account and `hackerchai.com` zone; the Global API Key is not placed in CI.
The source repository is public, as confirmed by the maintainer on 2026-10-11. Publication uses repository Secrets and adds no
manual approval step. `CI Required` reports all checks, and Publish enforces
their success. The main branch currently has no branch protection configured;
the earlier private-repository plan restriction no longer describes its visibility.
Do not change repository visibility or buy a plan automatically.

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

## Download site

Publish first confirms the R2 bucket `herdr-relay-dl` is writable, before
anything is published. After the crate, Worker, production probe and source
release succeed, `scripts/publish-download-site.py` verifies each of the four
CLI archives against the readiness manifest and uploads them, their checksums
and the manifest under `v{version}/` (immutable caching), then uploads
`latest/herdr-relay-update.json` last (five-minute caching). A retry uploads the
same bytes again. The bucket is public through the custom domain
`herdr-relay-dl.hackerchai.com`; source archives, accompanying Herdr builds and
CI logs stay on GitHub. The updater falls back to the source repository's GitHub
Releases when the site fails, so a site outage never blocks updates.
