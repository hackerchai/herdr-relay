# Interactive CLI acceptance — 2026-10-10

Scope: issues #33–#39, macOS arm64 and Ubuntu 26.04.1 arm64 in OrbStack.
The production Relay is https://herdr-relay.hackerchai.com. Official Herdr 0.9.3
is used on both endpoints. Separate XDG state roots isolate this acceptance run;
all interactive commands use their default profile without a `--profile` flag.

## Automated checks

- `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings`.
- `cargo test --all-targets --locked`: 50 tests passed on macOS and Linux.
- Worker: ESLint, `tsc --noEmit`, and all seven unit tests passed.
- Readiness manifest generation validated all four actual v0.1.2 CLI archives
  and their checksums locally. The manifest was not added to that historical
  release, whose CLI predates the new unattended service command.
- Necessary added tests cover command defaults, atomic sharing, host aliases,
  pairing links, release selection/archive validation, and service file rollback.
  No permanent e2e suite was added.

## Manual workflow

- `serve` discovered default and named Herdr sessions on both platforms.
  Selected two new Mac sessions and one Ubuntu session; install/start succeeded.
- New one-use links were pasted with bracketed paste into `pair` in both
  directions. Hostnames appeared without raw protocol identifiers.
- Native Herdr attached in both directions. Commands containing Chinese text
  wrote `MAC_TUI_中文_OK` on Ubuntu and `VM_TUI_中文_OK` on macOS.
- Detaching returned to the session picker. Contextual help opened correctly.
  A local host alias changed to `Ubuntu QA` and persisted after restarting the CLI.
- Bare invocation and `connect` both opened the same named-host browser.
  A non-terminal invocation exited with `TERMINAL_REQUIRED` and explicit guidance.
- Login autostart toggled off/on without changing paired grants or Herdr servers.
  Linux lingering was already enabled; repeated boot-mode setup is idempotent.
- A deliberately failing pending service executable on macOS produced
  `UPDATE_ACTIVATION` and automatically restored/restarted the previous service.

## Regression found during acceptance

A native socket EOF after detaching incorrectly marked a still-running Herdr
session offline until its socket generation changed. The focused regression
`closing_one_native_client_does_not_make_the_shared_session_offline` first failed
with `offline != online`, then passed on both platforms after removing that
incorrect state transition. Native server liveness still follows socket
resolution/generation checks and actual dial failures.

## Release verification

The new release, real GitHub self-update, and matching production Worker
publication are verified separately after the release pipeline completes.
macOS user-login startup is validated through its LaunchAgent definition and
service-manager lifecycle; the user's live desktop is not logged out or rebooted.
