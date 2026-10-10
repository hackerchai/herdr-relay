# Public binary release channel

Accepted 2026-10-10 after the maintainer explicitly chose a separate public binary
repository while retaining the private source repository. This supersedes the
private binary/update-channel portions of ADR-0013 and ADR-0014; MIT licensing
and public crates.io source distribution remain unchanged.

`hackerchai/herdr-relay-releases` contains distribution documentation and public
CLI release assets. `herdr-relay update` uses this channel anonymously by default.
The private source repository publishes only four CLI archives, their checksums,
the license and a readiness manifest after required CI, crate publication and
production deployment pass. The public repository receives no private Git history,
source archive, CI logs or automatically copied private changelog links. A draft
release becomes public only after every asset is uploaded. Completed public
releases are left unchanged on retries; corrected artifacts require a new version.

A publisher credential is stored only as an encrypted secret in the private source
repository. Clients never receive it. Alternate private release repositories can
still use the caller's own GitHub credentials through `--repository`.
