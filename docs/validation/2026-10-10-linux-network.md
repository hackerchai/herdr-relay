# Linux connector network acceptance — 2026-10-10

This follow-up to [candidate acceptance](2026-10-10-candidates.md) adds actual
network faults and recovery/ownership combinations for both Linux connector
architectures. Both runs passed. This is controlled manual acceptance, not a
permanent e2e suite. No product code, candidate binary or Worker deployment was
changed for this run.

## Environment and path

The frozen `0.1.0-dev` musl binaries were checked byte-for-byte against their
packaged binaries before starting. Both guests ran Ubuntu 24.04.4 and kernel
6.8.0-117-generic: aarch64 under Apple VZ, x86_64 under QEMU. Each used its matching
official Herdr 0.9.3 native client. These are VM results, not a new minimum-OS or
physical-hardware support claim.

Both connected to the real development Cloudflare Worker. A read-only deployment
query confirmed version `40b9997a-f39f-4101-85c6-6a0404831bc8` at 100% traffic.
The host Relay was the final macOS arm64 candidate. Linux arm64 operated the
isolated macOS Work target (x86_64 native Herdr under Rosetta); Linux x86_64
operated Build (arm64 native Herdr). Another authorized macOS device acted as the
replacement holder in each ownership check.

For deterministic faults, each guest connected through its own loopback-only
HTTP development override. The temporary proxy established verified TLS to the
actual Worker hostname; inner endpoint TLS and device authorization remained
active. Thus these fault runs exercise the real cloud and native path with an
explicit test transport at the connector's outer HTTP boundary. Earlier direct
WSS coverage remains in the candidate report. No GUI tool was used.

## Results

| Scenario | Linux arm64 | Linux x86_64 |
| --- | --- | --- |
| Repeated connector refusal and recovery | Passed; 4 disconnect observations | Passed; 4 disconnect observations |
| Input typed while offline | Test file never created after recovery | Test file never created after recovery |
| Blackhole without intentionally closing existing TCP | Detected after 29.888 s; recovered | Detected after 29.919 s; recovered |
| Native normal detach | Exit 0; termios restored | Exit 0; termios restored |
| Another device acquires target while old connector is offline | Old connector exits 1 with `TARGET_BUSY`; new holder survives | Same |
| Device revoked while waiting to reconnect | Old connector exits 1 with `NOT_AUTHORIZED`; fresh pairing restores access | Same |
| Ctrl-C during offline retry | Exit 130; termios restored | Exit 130; termios restored |

After each recovery, a unique command marker was checked through the actual
target server's independent pane-read API. An offline `touch` command used a
fresh nonexistent file on the macOS host; the file remained absent after each
recovery. The replacement holder processed a further marker after the old
connector exited, without a disconnect. Both holders detached normally with
terminal settings restored. These checks verify the outcome rather than treating
incoming terminal bytes alone as proof of a successful reconnect.

The blackhole scenarios began after active native input. Their approximately
30-second detection must not be relabelled as an isolated 75-second idle-silence
test: the stream also has a 30-second outstanding-credit deadline. These runs
did not instrument which timeout fired. They establish bounded disconnection
and successful fresh recovery under this fault, not a universal detection time.
The previous macOS 73-second observation remains separately recorded.

The [sanitized measurements](2026-10-10-linux-network-metrics.json) retain the
binary/native checksums, kernels, current deployment and per-scenario outcomes.
No secret keys, invitations, role tickets or terminal payloads appear there.

## Driver correction and retained evidence

The first attempted proxy path returned `RELAY_ERROR` before any fault scenario.
A minimized comparison showed direct CLI discovery succeeded, while the proxy's
first HTTP request succeeded and a subsequent request on the reused connection
received 403. The temporary proxy had rewritten only the first `Host` header.
Requesting connection close for ordinary HTTP exchanges fixed the driver;
WebSocket upgrades still retain their streaming connection. Both complete
successful runs used that corrected driver. This was not a product code fix or
a passed result from the initial failed attempt.

Exact temporary manual drivers and logs are retained privately under
`target/acceptance/2026-10-10/`:

- `hr-final-linux-network-cloud.py`: host orchestrator; takes `aarch64` or
  `x86_64`, verifies the installed binary, operates only the dedicated profiles
  and records native server markers.
- `hr-linux-network-rpc.py` and `hr-cloud-fault-proxy.py`: owned guest PTYs and
  bounded fault injection. The proxy records HTTP status/host-match diagnostics,
  never authentication headers or bodies.
- `hr-final-linux-{aarch64,x86_64}-network-cloud.log` and corresponding result
  JSON files; per-guest proxy logs and the sanitized deployment response.

Repeating this manual check requires the named isolated VMs, test profiles,
authorized grants, dedicated native Work/Build targets and development publisher;
it is not an installer or a test to run against an ordinary working session.
The driver leaves fresh valid grants after its revoke/re-pair checks.

After extracting results, all clients, replacement holders, guest proxies and
the temporary publisher were stopped. The newly started Build native server was
stopped; the pre-existing isolated Work server was preserved. Both VMs were
stopped, with their profiles/disks retained. The active Docker context was not
changed. GUI, upstream queue-bound and production-release gates remain open.
