# Issue tracker: GitHub

Issues and specs live in GitHub Issues for `hackerchai/herdr-relay`. Use the `gh` CLI and pass `--repo hackerchai/herdr-relay` to issue operations.

## Operations

- Read a ticket: `gh issue view <number> --comments --repo hackerchai/herdr-relay`. Include its body, labels and discussion when assessing it.
- List tickets: `gh issue list --state open --json number,title,body,labels,assignees --repo hackerchai/herdr-relay`; add the appropriate state or label filters.
- Publish a ticket: `gh issue create --title '<title>' --body-file <file> --repo hackerchai/herdr-relay`.
- Update a body: `gh issue edit <number> --body-file <file> --repo hackerchai/herdr-relay`.
- Comment: `gh issue comment <number> --body-file <file> --repo hackerchai/herdr-relay`.
- Apply/remove labels: `gh issue edit <number> --add-label '<label>'` or `--remove-label '<label>'`, with the repo flag.
- Close a ticket: `gh issue close <number> --repo hackerchai/herdr-relay`.

Write multiline bodies and comments to a temporary UTF-8 file and use `--body-file` to preserve formatting. The triage role mapping is in `triage-labels.md`; reuse matching labels and create missing mapped labels only when the authorized workflow needs them.

When a skill says “publish to the issue tracker”, create a GitHub issue. When it says “fetch the relevant ticket”, read the referenced GitHub issue including comments. Existing project documents remain authoritative for requirements and decisions; link them from tickets instead of maintaining competing copies.

## Pull requests as a triage surface

**PRs as a request surface: no.**

## Wayfinding operations

Used by `/wayfinder`:

- Map: one issue labelled `wayfinder:map`, containing Notes, Decisions-so-far and Fog.
- Child tickets: GitHub sub-issues of the map, labelled `wayfinder:research`, `wayfinder:prototype`, `wayfinder:grilling` or `wayfinder:task`. Use `gh issue create --parent <map-number>` or `gh issue edit <number> --parent <map-number>`, with the repo flag. If sub-issues are unavailable, maintain a task list in the map and `Part of #<map-number>` in each child.
- Blocking: use native GitHub issue dependencies where available (`gh issue edit <number> --add-blocked-by <blocker-number>`, with the repo flag). Otherwise maintain a `Blocked by: #<number>, ...` line. A child is unblocked when all blockers are closed.
- Frontier: select an open, unblocked and unassigned child, first in map order.
- Claim: assign the ticket to the driving developer before work; `--add-assignee @me` is appropriate when that developer is the authenticated user.
- Resolve: add the answer as a comment, close the child, then add a concise finding and link to the map's Decisions-so-far.
