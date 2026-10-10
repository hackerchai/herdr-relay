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
| `RELEASES_GITHUB_TOKEN` | Publish to `hackerchai/herdr-relay-releases`; minimum Contents: write on that repository |
| `CARGO_REGISTRY_TOKEN` | Publish `herdr-relay` to crates.io |
| `CLOUDFLARE_API_TOKEN` | Scoped Workers deploy, zone routes and billing read |
| `CLOUDFLARE_ACCOUNT_ID` | Deployment account |

The Cloudflare token has no billing-write permission. It is scoped to the existing
account and `hackerchai.com` zone; the Global API Key is not placed in CI.
The source repository and separate binary repository are public, as confirmed by
the maintainer on 2026-10-11. Publication uses repository Secrets and adds no
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

## Public binary channel

After the crate and production probe succeed, Publish generates the versioned
readiness manifest and invokes `scripts/publish-public-release.py`. It selects
only the four CLI archives named by that manifest and their validated checksums,
plus LICENSE. It uploads into a draft release, uploads the manifest last, then
publishes the draft. The tag belongs to the public documentation repository.
Source archives, accompanying Herdr builds, development history and CI logs stay
in the source repository rather than the binary repository. Completed public releases are
not overwritten by retries. A failed draft can be retried with the same tag.

`RELEASES_GITHUB_TOKEN` must belong to a publisher with Contents: write on the
public distribution repository. A fine-grained repository-scoped token or GitHub
App credential is sufficient; the source repository's built-in GITHUB_TOKEN
cannot publish across repositories. Store the credential as a source repository
secret, never in an artifact or the public repository. The current bootstrap uses
the maintainer's existing GitHub CLI credential in that encrypted secret.
