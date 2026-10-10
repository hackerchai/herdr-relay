# Domain docs

This repository uses a single-context layout. Its glossary is the existing root `CONTEXT.md`; decisions live in `docs/adr/`.

## Before exploring or changing the domain

1. Read root `CONTEXT.md` for the domain vocabulary.
2. Read `docs/adr/README.md`, then the ADRs relevant to the work. Distinguish confirmed decisions from proposals and unverified implementation details.
3. For requirements or behavior changes, read `REQUIREMENTS.md` and follow its links to the relevant design or validation evidence.

If a listed document is absent, continue without treating its absence as a blocker. Create domain documents when terms or decisions actually need recording.

## File convention

- `CONTEXT.md`: the project's existing glossary and domain context.
- `docs/adr/`: project-wide decision records and their index.
- No multi-context map is needed for the current layout.

Newer skill templates may refer to `GLOSSARY.md`. In this repository, those glossary reads and updates target `CONTEXT.md`; preserve one source of domain vocabulary rather than creating a parallel glossary.

## Consumer rules

Use the glossary's canonical terms in tickets, designs, code and tests. If a required concept is missing, resolve the terminology with `/domain-modeling` and update the existing context document.

Keep new implementation details in the appropriate design document or ADR. When a proposed change conflicts with an ADR, identify the affected ADR and explain why it should be reopened before treating the change as an accepted decision.
