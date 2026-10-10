# Four-platform candidate acceptance — 2026-10-10

Later on 2026-10-10, the maintainer accepted GUI testing and authorized a custom
upstream queue fix. See [current release status](../release-readiness.md) and the
[bounded server patch](../../patches/herdr/README.md). The observations below
remain the original unmodified-upstream / candidate evidence.

Issue: [#18](https://github.com/hackerchai/herdr-relay/issues/18).
**Result: candidate artifacts produced; full public-release acceptance remains
incomplete.** This record separates final-candidate reruns from earlier
implementation evidence. No permanent e2e suite was added.

## Provenance and installation

All four binaries identify as `herdr-relay 0.1.0-dev` and share this exact
build-input manifest SHA-256:

```text
ac934bc5eff5f4945c60186ca18bc1e651bd160cc2a1028b0d4d485f365ed0d8
```

The manifest covers Rust, Worker, protocol, module tests, build scripts and
locked tool/dependency configuration. There is no Git commit in this checkout;
`BUILD-INFO.txt` correctly records `uncommitted-source-snapshot`, Rust/Cargo
1.99.0, build OS, UTC build time and target. Packaging verifies the source hash
again after compilation. This makes the inputs traceable; it is not a claim of
bit-for-bit reproducible compiler output.

Artifacts are retained under `target/packages/` with matching `.sha256` files:

| Archive | SHA-256 |
| --- | --- |
| `herdr-relay-0.1.0-dev-aarch64-apple-darwin.tar.gz` | `f7e01b7cf1970cb19fc5fd8297d113cf0fa0fd80f53bdc301e2cf186dea5d3e9` |
| `herdr-relay-0.1.0-dev-x86_64-apple-darwin.tar.gz` | `a52000b0683130baf328d5a74afe4be71d849a1174fe51a0543e005487f56e87` |
| `herdr-relay-0.1.0-dev-aarch64-unknown-linux-musl.tar.gz` | `04956123e6ebe8fe74121fb96f8d94256583034a43ff67a9ea884280b32f51c8` |
| `herdr-relay-0.1.0-dev-x86_64-unknown-linux-musl.tar.gz` | `b0e46b32ba4d5e0e3614c5ca93eee66d43879a9003a08fa32b831647664c0e3b` |

The accompanying `herdr-relay-0.1.0-dev-source.tar.gz` contains explicit source
and documentation roots plus the build-input manifest, excluding credentials,
private profiles, `.wrangler`, dependencies, binaries and Git metadata. Its
adjacent checksum covers the final documentation snapshot. Neither the source
nor binary candidates have been publicly published or assigned a license.

Each archive was checksum-verified and installed. A disposable archive copy with
a modified `INSTALL.md` was rejected before replacing an existing binary; the
existing binary stayed unchanged. Installation, upgrade, stable service binary
copies and data-preserving removal are documented in [install](../install.md).

## Actual runtime coverage

| Candidate | Runtime exercised | Final candidate observations |
| --- | --- | --- |
| macOS arm64 | macOS 27, arm64 hardware | Native client, remote publisher, full LaunchAgent lifecycle, reconnect/native restart/permissions/disable cases |
| macOS x86_64 | macOS 27 on arm64 via Rosetta | x86_64 Relay and official x86_64 Herdr client/server; full LaunchAgent lifecycle and combined runtime recovery |
| Linux arm64 musl | Ubuntu 24.04.4, arm64 kernel 6.8.0-117, Apple VZ VM | Static candidate, systemd user lifecycle, simultaneous cross-OS roles and combined native/daemon/disable/revoke recovery |
| Linux x86_64 musl | Ubuntu 24.04.4, x86_64 kernel 6.8.0-117, QEMU VM | Same combined matrix with matching x86_64 Herdr and both cross-OS directions |

The native peer/client version was Herdr 0.9.3. The macOS x86_64 results are not
physical Intel Mac evidence. Linux results establish these guest environments,
not arbitrary distributions or physical x86 performance. Minimum supported OS,
kernel and terminal versions have not been established. No logout/login was
forced. Colima guests already had linger enabled; Relay did not change it.

All four final service lifecycles exercised repeated start with unchanged PID,
rejected installation while active, repeated stop preserving Herdr, restart with
an existing client recovering, repeated reinstall with opt-in login enablement,
and repeated uninstall preserving exact identity/grant/target file hashes.
Parentage was launchd PID 1 or the Linux user manager. Native detach returned 0
with termios restored. The test services ended uninstalled/stopped.

Both final Linux clients printed unique markers in the actual macOS Build pane;
their corresponding official native clients exited 0 and restored termios.
Earlier fresh cross-OS pairings and missing-user-bus/foreground fallback evidence
are in [Linux services](2026-10-10-linux-service.md). Those earlier checks are not
silently relabelled as fresh four-platform candidate pairings.

A further final-candidate matrix then ran independently on both Linux guests
through the development Worker. Each guest completed fresh pairing, two native
targets in parallel, an interactive same-target busy rejection, and a matching
Linux native client operating macOS while that guest was also publishing.
Server-side pane reads confirmed the markers in both directions.

Each guest's selected native server was gracefully stopped and restarted, then
crashed and restarted with the stale socket present. The driver verified the
old native process had actually exited and the new process had a different PID.
The other target stayed connected. Restarting its Relay publisher recovered both
targets; a command entered during the outage did not execute after recovery.
Target disable survived both native and daemon restart. After explicit enable,
both active targets processed fresh markers; revoking their connector ended both
and preserved the native servers. A fresh invitation restored target discovery.
All stopped/revoked clients restored termios.

A later [Linux connector network follow-up](2026-10-10-linux-network.md) used
these same installed candidates and the unchanged real development Worker.
Both architectures passed repeated connector refusal, blackhole recovery with
no offline-input replay, an old connector reporting `TARGET_BUSY` after a new
device acquired the target, revocation during retry followed by fresh pairing,
and Ctrl-C cancellation. All measured exits restored termios. The follow-up
records its loopback fault-injection boundary and approximately 30-second
data-active blackhole result separately from an idle-silence measurement.

The SSH-wrapped revoke command and subsequent observation took 5.270 s on ARM
and 5.402 s on x86_64. These are upper bounds including SSH and cloud
acknowledgement, not measurements from the local revocation commit to socket
close and not evidence for a five-second SLA. The test publishers ended stopped;
their private profiles and native configuration remain on the test disks.

## Final fault and terminal measurements

The final arm64 candidate ran a combined local-workerd Work/Build sequence:
native stop/restart, crash with stale socket, unsafe socket permissions then
recovery, daemon restart, and persistent target disable followed by another
native restart. Work alone went offline for native/permission faults; Build
stayed usable. Both recovered after the daemon restart. Disabled Work exited
with `TARGET_NOT_SHARED` and remained absent after native restart. Both terminal
settings were restored; no unrelated Herdr server was killed.

The same runtime sequence was repeated with the final macOS x86_64 Relay and
official x86_64 native Herdr client under Rosetta, including fresh pairing. Its
restarted Work servers also used the x86_64 Herdr binary. Stop/restart, crash and
stale socket, unsafe-permission rejection/recovery, daemon recovery of both
targets, and persistent disable all passed; the separate Build target remained
usable during Work faults. Disabled Work exited 1, normal Build detach exited 0,
and both restored termios. This remains an emulated Intel result on macOS 27.

A further x86_64 connector/publisher refusal sequence recovered without replay,
rejected a second interactive attach, revoked the active connector and restored
access only with a fresh invitation. After several preceding failures, frontend
exit took 15.331 seconds; an instrumented repeat separated this from access
revocation. The local incoming grant was absent and the native reservation was
already released at the first 54 ms sample. Frontend exit followed 23.742 seconds
later, matching its logged 23.7-second reconnect backoff. The transport had
stopped; error discovery waited for the next authorized access attempt. This
is an observable UX limitation: the offline supervisor can remain present during
existing backoff after revocation, with Ctrl-C available. It is not 24 seconds
of continued native access. Both runs restored termios and rejected the revoked
grant. No product change was made for this expected backoff behavior.

A separate final TCP-fault run covered repeated connector refusal, publisher
refusal and a half-open connection. The half-open fault was detected in
73.051 seconds after injection, consistent with the 75-second last-response
deadline. Recovery used fresh channels. Commands typed while offline did not
execute; fresh commands reached the same pane. Normal detach exited 0 and
restored termios.

The final slow/fast combined run suspended the Work connector while its native
pane generated bounded output. Work reached exactly 65,536 outstanding bytes,
remained bounded there, and its channel was absent by the 31.83-second sample.
Build processed eight independently checked markers without a disconnect;
observed response times were 125–148 ms. Work recovered after resume and both
clients detached with exit 0 and restored termios. The harness checked that the
load had actually started and that the credit window filled before accepting
the result.

| Process | Final render-load RSS, initial / peak KiB |
| --- | ---: |
| Publisher Relay | 8,512 / 8,832 |
| Suspended connector Relay | 7,728 / 7,728 |
| Active connector Relay | 7,792 / 7,824 |
| Native Herdr Work | 33,024 / 33,024 |
| Local workerd | 97,632 / 98,288 |

The [sanitized measurement file](2026-10-10-candidate-metrics.json) preserves
individual latency, throughput and pressure/RSS samples. These finite
observations do not resolve the upstream hard-bound limitation described below.

The self-host final candidate path completed a fresh invitation import, target
discovery, native Unicode input, same-target `409 TARGET_BUSY` without disturbing
the holder, active revocation, rejected revoked discovery and fresh re-pairing.
Revocation through CLI completion and observed client exit took 2.530 seconds in
that run. The revoked client exited 1 with termios restored; native Herdr
survived. Switching an active profile failed `SERVICE_BUSY`, a different origin
failed `PROFILE_MISMATCH`, forged proof failed `ACCESS_INVALID`, wrong protocol
failed `INCOMPATIBLE_PROTOCOL`, and an isolated missing-binding configuration
failed `RELAY_UNCONFIGURED`.

The matching development Worker passed a combined fresh-pairing/multiple-target/
bidirectional/revocation sequence with the final candidates: B operated A's
Build, C operated A's Ops, and A operated B's Personal concurrently. A second
interactive B or C attach to Build failed `TARGET_BUSY` and restored termios,
while its holder remained usable. Revoking B on A ended B's active channel in
2.588 seconds (CLI invocation through observed exit); A-to-B and C-to-A both
continued processing verified server-pane markers. B's new discovery was denied.
A fresh invitation restored B's access and it acquired the released Build target.
All normal clients detached with exit 0; revoked B exited 1; termios was restored
in each case. The native servers remained running.

Real self-host Cloudflare measurements with the final arm64 candidate:

| Measurement | Observation | Scope |
| --- | --- | --- |
| Input to native ANSI screen update | 20 samples after 4 warmups: p50 285.4 ms, p95 296.4 ms, min 275.5 ms, max 303.5 ms | Reconstructed screen, not monitor refresh; single endpoint location |
| 2 MiB raw RGBA image | 9.549 s; useful payload 214.5 KiB/s | SHA-256 exact at receiver; one bounded burst transfer |
| Encoded terminal output for that image | 2,802,693 bytes, 286.6 KiB/s | Includes base64/control overhead; not ciphertext rate or a sustained quota violation |
| Image SHA-256 | `f76d64ba1f33a68b5480e8c65c09764fd53f219676141504e5f932170b085599` | 1,024 × 512 RGBA, 2,097,152 decoded bytes |

The latency harness was corrected to interpret ANSI differential updates instead
of searching raw chunks for complete strings. These harness failures are not
included in the performance samples. The throughput result includes a permitted
initial burst and measures native output, so it must not be extrapolated as a
sustained ciphertext rate. Both successful probes detached cleanly and restored
the terminal. Payload-level paste/mouse/resize/clipboard/image evidence and the
TLS idle-flush regression are in [terminal acceptance](2026-10-10-terminal.md).

The final Worker source is deployed as development version
`40b9997a-f39f-4101-85c6-6a0404831bc8` and self-host version
`61f4cb6f-e29f-4f80-99cb-653f4a01add8`. Both deployments retain independent v3
Admission/AuthGate/Device/Room namespaces and restrict test-device admission.
On the final self-host version, a further native session stayed idle from
2026-10-09 23:42:22.611 to 23:44:37.630 UTC (135 seconds), then passed 50,444-byte
paste checksum, SGR mouse input, resize, controlled clipboard payload and
76,800-byte RGBA image checksum checks. It detached with exit 0 and restored
termios. Thus the final deployment also crossed the 120-second original lease;
its endpoint did not need a keystroke to complete the initial image surface.

Provider metrics for that final 135-second idle window included all four DO
classes (two AuthGate objects). The [sanitized rows](2026-10-10-candidate-cloud-metrics.json)
preserve the returned units and invocation counts:

| DO class | Active seconds | CPU seconds | GB-seconds | SQL rows read / written |
| --- | ---: | ---: | ---: | ---: |
| DeviceRelay | 0.821831 | 0.014127 | 0.105194368 | 100 / 16 |
| Room | 0.144391 | 0.003906 | 0.018482048 | 16 / 4 |
| Admission | 0.527554 | 0.007990 | 0.067526912 | 178 / 26 |
| AuthGate, two objects | 0.065689 | 0.001293 | 0.008408192 | 8 / 0 |
| Total reported | 1.559465 | 0.027316 | 0.199611520 | 302 / 46 |

The invocation dataset reported 25 Device hibernation events, five Device
alarms, five Room RPCs, nine Admission RPCs, three Admission alarms and two
AuthGate alarms, all successful. Maximum Device isolate memory was 2,334,689
bytes. This is a sampled idle baseline for one publisher/attach; it excludes
account-wide Worker costs, storage footprint and other traffic. It is neither
a production bill nor a process-private memory maximum.

The [v3 admission record](2026-10-10-admission.md) contains controlled quota,
shutdown, retention and cloud-idle results, including delayed global coordinator
cost samples. The earlier pre-v3 [hibernation sample](2026-10-10-hibernation.md)
must not be used as a complete estimate for this deployment.

## Necessary checks and remaining gates

All 38 Rust tests passed on macOS arm64 and Linux arm64 with the final source.
Seven TypeScript tests and type checking passed; formatting and strict Clippy
passed. The new transport regression uses real TLS and an idle native source;
the two admission tests use SQLite transactions and persistence. No e2e suite
was added. Reproduce the overall workflow with the [manual checklist](../manual-acceptance.md).

Public release remains blocked by these explicit gaps:

- Actual GUI terminal rendering, user mouse interaction and real desktop
  clipboard acceptance are incomplete. The Ghostty computer-use request was
  rejected by the tool safety check; protocol captures are not substituted for
  that GUI result.
- Every recovery/race combination has not been repeated on every host/connector
  architecture and terminal. Rosetta/VM coverage and the tested OS versions must
  remain visible; no unsupported minimum-version promise is made.
- Herdr's reliable control queue has no enforced bound in the pinned upstream
  source. An [isolated slow-reader run](2026-10-10-upstream-control-queue.md)
  recorded 23,856 KiB of additional native RSS while consuming throughout 65
  seconds; it plateaued only after the fixed producer finished. This is upstream
  evidence, separate from candidate Relay measurements. The [bounded Relay load
  observations](2026-10-10-flow-control.md) do not prove a full-chain hard memory
  maximum. Cloudflare's private send-buffer size is also not directly observed.
  These resource requirements remain open.
- [#19 decisions and #20 release](../release-readiness.md) are pending. No
  production domain, public budget, license or support promise has been invented.

Earlier evidence for old-release/new-holder ownership, revocation while
reconnecting, alias rejection and hibernation/occupancy interactions remains
linked in [exclusivity](2026-10-10-exclusivity.md), [reconnect](2026-10-10-reconnect.md),
[revocation](2026-10-10-revocation.md) and [hibernation](2026-10-10-hibernation.md).
These are useful implementation results but do not justify marking the entire
#18 matrix complete by themselves.

## Retained evidence and cleanup

Selected local logs are preserved privately under
`target/acceptance/2026-10-10/`; source-controlled JSON files above contain only
sanitized measurements. Temporary drivers remain outside the repository and
are not an e2e test suite. All test user services were uninstalled, foreground
test publishers were stopped, and the two isolated Colima VMs were stopped after
extracting artifacts and results. Their disks/profiles remain available. The
controlled Cloudflare Workers and namespaces are retained for continuation;
they have test-device allowlists and have not become the default public service.
Local native macOS test sessions are separate from user work and remain intact.
