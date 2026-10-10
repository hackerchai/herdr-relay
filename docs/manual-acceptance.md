# Manual candidate acceptance

Use this checklist for each candidate source manifest, on both a controlled
hosted Relay and a separately configured self-hosted Relay. It is a manual
regression entry point, not an e2e suite. Record actual observations; leave
unavailable combinations pending rather than copying a result from another OS.
Keep dated run records locally, outside the tracked documentation.

## Record the environment

Preserve the archive checksum, `SOURCE-MANIFEST.sha256`, `BUILD-INFO.txt`,
`herdr-relay --version`, `uname -srm`, native Herdr version/checksum, terminal
name/version, OS login policy and Worker deployment version. Use only disposable
Herdr sessions/profiles. Capture error codes and timing without private keys,
invitation text, proof headers or role tickets. Raw terminal captures can contain
private data; use deterministic acceptance content in isolated panes.

Verify the archive and its inner manifest, install it into a fresh private
directory and run `--version`. Repeat with an existing installation; tamper with
one file in a disposable extracted copy and confirm installation is rejected
before the existing binary changes. Follow [installation](install.md) for service
upgrade and [self-hosting](self-hosting.md) for a fresh Worker deployment.

For necessary module checks, run from the source snapshot:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cd relay
npm ci
npm run check
npm test
```

## Pair, publish and connect

Choose the candidate origin and absolute CLI/Herdr paths; replace the example
IDs with `identity show` and `share list` output. On each of two devices, use a
private profile and start your own two named Herdr sessions before sharing them.
Relay must not start the native server as a side effect.

```sh
herdr-relay --profile /private/profile-a identity init
herdr-relay --profile /private/profile-a share add --session work --name Work
herdr-relay --profile /private/profile-a share add --session build --name Build
herdr-relay --profile /private/profile-a serve --relay --origin https://YOUR-RELAY
```

From another shell on A, run `pair create`. On B, run `pair accept` and paste
the invitation at its hidden prompt. For isolated same-machine profiles, pipe
the output directly; never place an invitation in argv or a report.

```sh
herdr-relay --profile /private/profile-a pair create |
  herdr-relay --profile /private/profile-b pair accept
herdr-relay --profile /private/profile-b targets --origin https://YOUR-RELAY --host A_ID
herdr-relay --profile /private/profile-b connect --origin https://YOUR-RELAY \
  --host A_ID --target WORK_ID --herdr /absolute/path/to/herdr-0.9.3
```

1. Confirm input in the intended server pane using a unique harmless marker.
   Native detach must exit 0, restore `stty -g` and leave the server/pane alive.
2. Connect Work and Build simultaneously. A second Work connector must return
   `TARGET_BUSY` while the original remains usable. Repeat with a different
   authorized device and a duplicate path/session alias.
3. Pair independently in the opposite direction and operate both devices at
   once. Revoke B on A with `peers revoke B_ID --origin ...`; A-to-B access and
   another authorized peer must survive. B's existing and new access must stop.
   Reimporting an old receipt must not restore it; a fresh invitation must work.
4. In an actual supported terminal, verify Unicode/combining text, bracketed
   paste, resize, mouse down/up, clipboard and graphics. Compare the same pane
   directly through native Herdr and through Relay. Protocol byte equality is
   useful evidence but does not replace visible rendering and OS clipboard checks.

## Combine recovery with exclusivity

Record the original native server PID, target ID and unrelated connection before
each fault. Operate only the dedicated process/socket; never kill by broad name.

1. Interrupt connector traffic, retain refusal for multiple retries, and type a
   harmless unique marker while offline. Restore traffic: a fresh connection must
   recover and the offline marker must not execute. Cancel a separate recovery
   using Ctrl-C; expect exit 130 and restored termios.
2. Blackhole packets without closing TCP. Measure detection against the 75-second
   silence deadline and verify recovery. Interrupt publisher traffic separately.
3. While the first connector is offline, let a second acquire the target. The
   first must stop with `TARGET_BUSY`; old release/close events must not free the
   new holder. Revoke a waiting connector and confirm it does not reconnect.
4. Stop/restart only the selected native server; repeat with a crash and stale
   socket. The same saved target must recover with a new runtime generation;
   another target stays usable. Unsafe socket permissions must fail closed.
5. Restart the Relay daemon. Identity, grants and target IDs persist; existing
   clients recover without re-pairing. Disable the target, restart its native
   server and daemon, and confirm it remains disabled until explicit enablement.
6. Suspend only one connector while its pane produces bounded test output.
   Operate another target throughout. Record credit pressure and RSS for Relay,
   Herdr and runtime processes. The stalled channel must close after no progress;
   pong must not extend credit. See the [flow-control protocol](../protocol/access-v1.md).
7. Leave active channels idle for more than 120 seconds. Confirm hibernation
   through Worker metrics/status, then operate both targets and repeat competing
   attach/release. An expired or stale reservation must not gain capacity.

Use the local development transport plus a disposable TCP fault proxy for
deterministic faults; also exercise actual cloud restart/revoke paths. Do not
capture a real user's clipboard or use an unbounded load generator. For latency,
time key submission through reconstructed native screen output, rather than
searching individual ANSI delta chunks. For throughput, report useful bytes,
encoded bytes, duration and payload checksum separately.

## Services, quotas and rollback

Run every applicable service step on all four binaries, with persistent private
profiles outside `/tmp`. Check repeated start keeps the same PID; install while
loaded fails; start-at-login is explicit; stop/restart recovers an existing client;
reinstall/uninstall is idempotent; profile data hashes and Herdr survive. Record
launchd/systemd parentage, missing-manager fallback and current linger setting.
Do not log out an active user session merely to test it without authorization.

Lower one quota at a time in an isolated Worker. Test new identities, slow
unsigned bodies, pending and active channel pools, per-device limits and global
publisher/time budgets. Confirm the exact error and Retry-After, unaffected
existing connections and recovered capacity after release/expiry. Reconstruct
objects/configuration without resetting their stored counters. With admission
off, inspect signed operator status and verify active routes close. Restore the
same namespace/configuration and existing local grants. See
[operations guide](operations.md).

Preserve the previous candidate archive and sanitized configuration. Roll back
by stopping the user service, installing that verified binary, reinstalling its
service with the same profile/origin and starting it. A Worker rollback must
retain the existing DO namespaces/migrations; never delete namespaces to reset
usage. Record the actual compatibility of the rollback version before promising
it to users.

## Exit record

For every row, save candidate hash, host/connector architecture, origin/deployment,
scenario, result, exit/termios status, timing and diagnostic artifact location.
Distinguish final-candidate reruns from earlier implementation evidence. Missing
GUI, resource, cost or platform coverage stays open. Public release additionally
requires the maintainer decisions in [#19](https://github.com/hackerchai/herdr-relay/issues/19)
and the deployment/handoff in [#20](https://github.com/hackerchai/herdr-relay/issues/20).
