# Herdr Relay agent instructions

## Local documentation

Keep internal agent guidance, ADRs, plans and validation records in `.local-docs/`.
This directory is ignored by Git and must stay out of commits and release packages.
Write tickets, PRs, plans and ADRs in English.

The local documents below are optional on a fresh clone. Read them when present;
otherwise use the tracked guides, source and GitHub Issues without blocking work.

### Issue tracker

Track issues and specs in `hackerchai/herdr-relay` GitHub Issues. Before reading or publishing tickets, read `.local-docs/agents/issue-tracker.md` when available.

### Triage labels

Use the five default triage roles: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human` and `wontfix`. Before triaging or changing triage labels, read `.local-docs/agents/triage-labels.md` when available.

### Domain docs

Before domain exploration or terminology/decision changes, read `.local-docs/agents/domain.md` and the relevant records in `.local-docs/adr/` when available. Check current behavior against `README.md`, `protocol/` and the implementation. Keep new decision and validation records under `.local-docs/`.
