# Bootstrap verification, 2026-10-10

This records historical preparation for [ticket #1](https://github.com/hackerchai/herdr-relay/issues/1).
For the subsequent implementation and validation, see
[local CLI acceptance](2026-10-10-local-cli.md). The remaining paragraphs describe
the bootstrap state at the time of this record.
The ticket remains incomplete. The executable contains only an unavailable
message; no Rust bridge, product test suite, or real native TUI acceptance has
been implemented.

## Environment and inputs

- macOS 27.0, arm64.
- `rustc 1.99.0 (b940084d7 2026-09-28)` and Cargo 1.99.0.
- Default installed Herdr: 0.8.2; left unchanged.
- Probe binary: official `herdr-macos-aarch64` from
  [Herdr v0.9.3](https://github.com/herdrdev/herdr/releases/tag/v0.9.3),
  reporting `herdr 0.9.3`.
- SHA-256 matched the release asset digest:
  `5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157`.

## Executed checks

`cargo check --locked` and `cargo fmt --check` passed on the bootstrap. They
validate the build setup and formatting only.

The existing `scripts/probe-herdr-0.9.3.py` was run with the verified binary.
Its result was:

```json
{
  "two_native_clients_bootstrap": true,
  "passive_attach_preserves_geometry": [40, 119],
  "focus_moves_geometry_control": [24, 79],
  "both_clients_input_and_receive_updates": true,
  "peer_disconnect_restores_survivor_geometry": [40, 119],
  "disconnect_keeps_server_and_survivor": true,
  "separate_instance_socket_isolation": true,
  "created_processes_exited": true
}
```

These are upstream protocol checks through synthetic native endpoints. They
do not establish product multi-client behavior, native TUI usability, terminal
restoration, the Rust bridge, or remote connectivity. The probe uses two
clients to observe upstream behavior; Relay must enforce one active client
per instance as required by ADR-0012.

## Next implementation boundary

The proposed tests observe CLI commands, real Unix socket byte streams, and
native Herdr subprocess/terminal lifecycle. Confirmation of those boundaries
was initially requested by the invoked TDD skill. The maintainer subsequently
instructed the agent to choose the boundaries without further confirmation,
keep only necessary tests and use manual overall validation instead of e2e
tests. That instruction removed the bootstrap blocker.

After confirmation, ticket #1 still needs its full red/green implementation
and isolated real-TUI evidence, including Unicode, resize, path ownership,
environment isolation, timeout/version errors, cancellation, terminal
restoration, and preservation of Herdr and its pane processes.
