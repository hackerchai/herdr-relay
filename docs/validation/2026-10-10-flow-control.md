# Flow control and device quotas — 2026-10-10

Issue: [#11](https://github.com/hackerchai/herdr-relay/issues/11).
Environment: macOS arm64, Herdr 0.9.3, debug Rust endpoints, local workerd
1.20261006.1. These are controlled manual checks, not an e2e suite or production
latency/capacity promises. Real Cloudflare hibernation/cost evidence belongs to #12.

## Implemented bounds

A target has exactly one pending or active attach. A publisher permits eight
attach channels, four combined pair/control channels and four unauthenticated
channels. Self-hosted values can be reduced within those bounds. Admission and
release use persisted reservations; the host's inode lock independently protects
native instances. Pair channels remain pending until closure; other channels
leave the pending count only after publisher confirmation following inner TLS,
BIND and a fresh local grant check.

Each direction permits a 64 KiB outstanding ciphertext window, 16 KiB DATA,
256 KiB/s default DATA rate (512 KiB burst), and 150 DATA/CREDIT messages/s
(300 burst). Rust paces at 95% of the advertised rate. Self-hosted byte rates
can be reduced to 16 KiB/s. Duplicate CREDIT cannot extend the 30-second progress
deadline, exceed sent bytes or replenish message budget. Offsets use exact u64
arithmetic. Pong cannot keep a stalled data channel alive.

The window was reduced from the design's proposed 256 KiB. Eight configured
targets imply at most eight attaches, so the proposed sixteen attach quota was
unnecessary. Rust duplex buffers are 16 KiB; TLS and WebSocket buffers are
separately bounded. The design's 16 MiB total application-buffer target is not
advertised as a separate allocator or process-RSS cap.

## Manual observations

- An isolated Worker with `MAX_DEVICE_ATTACH=1` accepted Work, then rejected
  Build with `429 QUOTA_EXCEEDED`. Work remained usable. After normal detach,
  Build attached and accepted input: the failed reservation had not leaked its
  Room slot. Both clients exited 0 and restored terminal settings.
- Four incomplete control channels were accepted; the fifth was rejected.
  Repeated owner release was idempotent and a new channel immediately succeeded.
  After 32 seconds without redeeming that channel, the pending count was zero.
- A real Work client's connector Relay process was suspended while its existing
  pane printed 2,000-character lines every 10 ms. Build remained active. Work's
  outstanding ciphertext reached exactly 65,536 bytes, stayed there, and was
  absent by the 31.81-second observation. The slow-consumer counter advanced once.
  Build never disconnected and accepted eight markers. Work reconnected after
  resume. Both detached normally and restored terminal settings.
- A separate Work load emitted 2,000 OSC 52 control messages containing 8 KiB
  each at 15 ms intervals. The connector was suspended before output began; its
  owned native client was terminated before resuming Relay to avoid applying
  buffered clipboard operations. Build continued accepting markers. Work again
  reached 64 KiB and closed by the 31.86-second observation. This bounded stress
  run does not prove an upstream hard queue limit.

RSS from `ps`, in KiB, over approximately 42-second sampling windows:

| Process | Render: initial / peak | Control: initial / peak |
|---|---:|---:|
| Publisher Rust | 14,560 / 14,592 | 15,024 / 15,040 |
| Suspended connector Rust | 13,104 / 13,104 | 13,504 / 13,504 |
| Active connector Rust | 13,056 / 13,120 | 13,136 / 13,152 |
| Herdr Work | 32,848 / 33,008 | 36,112 / 39,488 |
| Herdr Build | Not sampled | 24,400 / 24,432 |
| Local workerd | 120,384 / 120,912 | 124,048 / 124,544 |

Render-run marker response was 132–143 ms. The control run checked native output
progress and used Herdr's independent JSON API to confirm each marker was
processed: 133–146 ms. These are polling-harness observations, not wire RTT or
precise TUI rendering benchmarks. Early harness attempts failed because native
ANSI differential updates split text; those attempts are not counted as product
failures or included in this table.

Publisher-authenticated `/status` recorded `sent - ack`, peak window use and
bounded close counters. Workers has no documented `bufferedAmount`/send-queue
size property. Thus 64 KiB is application pressure; local workerd RSS is a process
measurement. Neither directly measures Cloudflare's private network send queue.
The implementation never substitutes an unlimited `send()` queue for credit.

## Upstream limitation and retained reproduction

Herdr's pinned source at
[`7b116c05`, client_transport.rs](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/client_transport.rs)
uses a single pending render and a small ordered render queue, but
`ClientWriterQueue::send_control` pushes into an unbounded `VecDeque`. Clipboard
writes and notifications use this reliable path. Relay cannot safely inspect or
drop arbitrary native frames to repair it.

The suspended control run grew Herdr RSS by 3,376 KiB before stabilizing. It did
not show unlimited growth, expose queue lengths, or establish a maximum for
other rates. A later [isolated slow-reader investigation](2026-10-10-upstream-control-queue.md)
kept the native socket consuming throughout 65 seconds: Herdr RSS increased by
23,856 KiB across two output waves, then plateaued after the fixed producer
finished. The connection remained alive. The upstream 30-second write timeout
belongs to observer mode, not this native endpoint path. Neither a no-progress
disconnect nor the render queue bound establishes a reliable-control byte limit.
Full-chain bounded-memory release acceptance remains open until the upstream
queue has an enforced bound or further evidence supports an explicitly accepted
limit. Relay's stable RSS does not satisfy that requirement.

To repeat manually, use two isolated Herdr sessions, attach both through Relay,
suspend only the first connector Relay PID, and emit this from its existing pane:

```python
import base64, sys, time
payload = base64.b64encode(b'x' * 8192).decode()
for _ in range(2000):
    sys.stdout.write('\x1b]52;c;' + payload + '\x07')
    sys.stdout.flush()
    time.sleep(.015)
```

Sample publisher, both connector and Herdr PIDs plus local workerd RSS every five
seconds; send a unique marker through the second client at each sample. Record
signed publisher pressure status. Verify the first channel closes after 30
seconds without advancing credit and the second remains attached. Terminate
only the first connector's owned native client before resuming Relay; never stop
the target server or run this against a real working clipboard. Vary content,
rate and duration in bounded increments to investigate the upstream queue.
Do not infer queue size from RSS alone.

## Necessary automated checks

36 Rust tests cover existing module boundaries, including bidirectional 1 MiB
stream transfer and a paused-time peer that answers Ping but never returns
credit. Four TypeScript tests include forged/duplicate credit, overflow,
oversize/empty DATA, control floods, restored token state and invalid quota
configuration. Type checking and strict Clippy passed. No e2e suite was added.

References: [Workers WebSockets](https://developers.cloudflare.com/workers/runtime-apis/websockets/),
[Hibernation WebSockets](https://developers.cloudflare.com/durable-objects/best-practices/websockets/).
