# Upstream reliable-control queue — 2026-10-10

Later on 2026-10-10, the maintainer accepted GUI testing and authorized a custom
upstream queue fix. See [current release status](../release-readiness.md) and the
[bounded server patch](../../patches/herdr/README.md). The observations below
remain the original unmodified-upstream / candidate evidence.

Related release gates: #11, #16 and #18. **The pinned upstream has no enforced
reliable-control queue bound. A finite slow-reader experiment demonstrates
continued native-process memory growth while the connection remains alive.**
This is a retained manual investigation, not an e2e suite or a Relay regression
test. No Herdr source or released binary was modified.

## Source path

Source revision: Herdr `7b116c05bfda646af39d2524c54e70c751f57ee8`, version 0.9.3.

A fresh GitHub check on 2026-10-10 still returned
[v0.9.3 as the latest release](https://github.com/herdrdev/herdr/releases/tag/v0.9.3).
The default branch at
[`2563803dca97c040beaf3dc3acdcb5a3221b4238`](https://github.com/herdrdev/herdr/blob/2563803dca97c040beaf3dc3acdcb5a3221b4238/src/server/client_transport.rs)
(committed 2026-10-08) also retained the same unbounded `control.push_back(data)`
path. No released or current-main queue bound was available to adopt at this
check; this does not claim to review every unmerged upstream branch or PR.

- [`ClientWriterQueue::send_control`](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/client_transport.rs#L310)
  appends serialized messages to a `VecDeque` without a count or byte limit.
  Pending render frames have separate bounds; those do not bound control data.
- [Clipboard forwarding](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/headless/notifications.rs#L334)
  produces `ServerMessage::Clipboard`; the foreground-client path sends the
  serialized frame through the reliable control writer.
- [`write_client_stream`](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/platform/unix_common.rs#L68)
  uses blocking `write_all` when there is no socket write timeout.
- The [30-second observer timeout](https://github.com/herdrdev/herdr/blob/7b116c05bfda646af39d2524c54e70c751f57ee8/src/server/client_transport.rs#L1071)
  is configured for `ObserveTerminal`. The native endpoint hello used by this
  investigation and Relay's client path does not select that observer mode.
  Even that timeout measures lack of write progress, not queue occupancy.

Relay's finite ciphertext window prevents its own unlimited buffering, but can
push pressure back onto this native writer. Disconnecting after **no** credit
progress does not establish a native queue bound for a peer that continues to
consume slowly. Dropping arbitrary native frames or replacing the official
native protocol is not a transparent repair.

## Controlled observation

Environment: macOS 27 arm64; official `herdr-macos-aarch64` 0.9.3, SHA-256
`5173a3e0ae42d5d1ab7ebfa5d5e6329f7c3d23f8e1a3677c7ce3231da2884157`.
The server, pane and raw protocol client belonged to a fresh temporary directory.
There was no Relay, cloud deployment, GUI client or OS clipboard application in
this isolated upstream experiment.

The client completed endpoint hello, received snapshot/surface state, selected
focus and observed a warmup `Clipboard` variant. Its receiving thread was then
stopped while retaining the socket. A manual reader consumed at most 4,096 bytes
every 250 ms (16 KiB/s nominal ceiling). The pane emitted two waves of 800 unique
OSC 52 sequences with 8,192 decoded payload bytes each, 20 ms requested spacing,
and a five-second inter-wave pause. File I/O and scheduling made the actual
producer slower than the requested spacing. Total requested payload was
13,107,200 bytes (12.5 MiB); this is not a measurement of queue occupancy.

| Elapsed seconds | Producer writes completed | Consumer bytes read | Herdr RSS KiB |
| ---: | ---: | ---: | ---: |
| 0.001 | 0 | 116 | 22,416 |
| 20.065 | 695 | 307,316 | 35,120 |
| 25.078 | 800 | 385,140 | 36,432 |
| 30.106 | 873 | 462,964 | 37,248 |
| 40.148 | 1,219 | 618,612 | 41,504 |
| 50.172 | 1,568 | 774,260 | 45,792 |
| 55.190 | 1,600 | 852,084 | 46,208 |
| 60.191 | 1,600 | 929,908 | 46,208 |

The connection and server survived the 65.053-second experiment. RSS increased
from the pre-producer baseline of 22,352 KiB to 46,208 KiB, a 23,856 KiB increase.
It grew during both output waves and plateaued after production ended. The
[sanitized measurements](2026-10-10-upstream-control-queue-metrics.json) preserve
every five-second sample. RSS includes the whole server, not just the queue;
the experiment neither instruments exact queue length nor proves infinite growth.
The source establishes the missing bound, and the run establishes an observable
slow-consumer consequence beyond a 30-second completely stalled case.

The initial 50-second driver did not allow the bounded producer to finish and
failed its completion assertion. It is excluded from the successful sample.
The repeated run extended the observation to 65 seconds and made the producer's
progress-file update atomic. No product change was involved. Cleanup closed the
test client, waited for only its server to stop and removed the temporary tree.

## Manual reproduction

Use the existing [isolated protocol probe](../../scripts/probe-herdr-0.9.3.py)
helpers with the official 0.9.3 binary; do not attach to a working user session.
The exact local driver and log are retained privately under
`target/acceptance/2026-10-10/hr-upstream-slow-reader.py` and
`hr-upstream-slow-reader.log`. It accepts the known local binary path in the
driver; adjust that path and the repository probe path when moving machines.
Its fixed workload and lifecycle are:

1. Create a fresh `TemporaryDirectory`, `Server` and one pane. Connect `Client`
   with 100 columns and 24 rows, and wait for `ready()`.
2. Send `vi(18) + b'\x01'` (focus). Emit a small OSC 52 warmup from the pane and
   wait until the client's decoded messages include variant 5. The raw helper
   records protocol variants; it never invokes a clipboard program.
3. Set `client.closed = True`, join its reader thread and assert it stopped.
   Keep the socket open with a 10 ms read timeout. Discard raw reads of up to
   4,096 bytes every 250 ms for 65 seconds; fail on EOF or server exit.
4. Start this bounded producer inside the pane, recording its counter through
   an atomic temporary-file replacement after each write:

   ```python
   for wave in range(2):
       for i in range(800):
           n = wave * 800 + i + 1
           payload = str(n).zfill(8).encode() + b'x' * 8184
           os.write(1, b'\x1b]52;c;' + base64.b64encode(payload) + b'\x07')
           progress.with_suffix('.tmp').write_text(str(n))
           progress.with_suffix('.tmp').replace(progress)
           time.sleep(.02)
       if wave == 0:
           time.sleep(5)
   ```

5. Sample only the owned server PID with `ps -p PID -o rss=` every five seconds.
   Abort if RSS exceeds baseline by 96 MiB. At completion assert the producer
   reached 1,600, preserve the samples, and close the client/server in `finally`.

This differs intentionally from the earlier [suspended Relay experiment](2026-10-10-flow-control.md):
the receiver keeps making progress. An upstream fix should enforce a serialized
control-byte budget, preserve reliable ordering or terminate the affected
client explicitly on overflow, and release pending data on closure. Those are
repair criteria, not a claim that a patch exists or that a new native version
has passed compatibility testing. Full-chain bounded-memory release acceptance
remains open until the supported upstream path meets that requirement.
