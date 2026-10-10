# Bounded native queue acceptance — 2026-10-10

The custom Herdr `0.9.3-custom.relay-control-bound-1` build fixes the missing
reliable-control queue bound reproduced in the [original investigation](2026-10-10-upstream-control-queue.md).
It is a maintained local patch to pinned upstream 0.9.3, not a claim that upstream
has merged or released the fix. [Source and build instructions](../../patches/herdr/README.md).

Three necessary module regressions were written and observed failing before each
corresponding change: the 257th pending message was accepted; allocated vector
capacity was not charged; and closing the queue did not interrupt the actual
native stream. All three pass with the patch. All 32 upstream transport tests
also pass on macOS arm64. No permanent e2e suite was added.

The original finite slow-reader workload was repeated with a fresh isolated
patched server, a real native endpoint and no Relay or OS clipboard application.
It emitted 1,600 OSC 52 messages in two waves, 8,192 decoded bytes each. The peer
read at most 4,096 bytes per 250 ms. It observed EOF at 7.950 seconds; production
continued to all 1,600 messages. RSS was 22,560 KiB before production and peaked
at 29,840 KiB, a 7,280 KiB increase. It stayed near 29,824 KiB throughout the
second wave. The earlier unmodified run grew by 23,856 KiB and remained connected.
These are finite whole-process RSS observations, not exact queue allocation
measurements; the allocation bound is enforced by the patched queue itself.

After the 65-second observation, a new native client attached and submitted a
unique command whose output was confirmed through the independent pane API.
The native server survived the slow client's termination. The probe closed only
its own clients/server and removed its temporary session directory.

[Sanitized measurements](2026-10-10-bounded-herdr-metrics.json) include the binary
SHA-256 and every five-second RSS sample. The exact manual driver and log remain
private under `target/acceptance/2026-10-10/hr-bounded-slow-reader.*`.

The maintainer separately instructed that GUI testing should be considered
passed. This is recorded as human acceptance; it does not relabel the native
protocol probe as a new GUI automation run. Existing unmodified Herdr sessions
must be restarted with the bounded binary to receive the queue fix.
