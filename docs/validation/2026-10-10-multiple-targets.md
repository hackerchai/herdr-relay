# Ticket #6: persistent target management

2026-10-10, same macOS arm64 machine and controlled Cloudflare deployment as
[the native acceptance](2026-10-10-remote-native.md). No Worker change was
needed for this slice. Official Herdr 0.9.3 ran two isolated named sessions,
`work` and `personal`, under `/tmp/hr-multi-native/config/herdr`.

A saved Work as target `92100968f17deb038168da99520b6b5a`. B attached through
public WSS using its existing grant. While B remained attached, A added
Personal as `78848c46746be2fed218bd658e547633`; already-granted C immediately
saw both targets and attached to Personal without another pairing.

Distinct native commands produced `WORK_978834087160958` only in Work and
`PERSONAL_978834087160958` only in Personal. Both official clients ran at once.
The local API observations agreed with the rendered clients; neither marker
appeared in the other session.

Adding Work again through its explicit socket path returned `TARGET_EXISTS`.
Disabling Work persisted its disabled state, closed B's native connection
(exit 1 after closure), and removed Work from the encrypted target list. C
remained attached to Personal and successfully printed
`PERSONAL_STILL_ACTIVE_978834087160958`. C's normal detach returned exit 0.
Both clients restored their exact prior terminal flags, and both Herdr servers
and panes survived.

After a Relay service restart, Work remained disabled and Personal retained
its ID. Enabling Work restored its original ID. Removing Personal left Work
published; trying the removed target returned `TARGET_OFFLINE`. Reusing a
display name returned `TARGET_NAME_CONFLICT`, adding an absent session returned
`TARGET_OFFLINE`, and removing an unknown target returned `TARGET_NOT_FOUND`.
No operation silently selected another session.

The necessary storage-boundary test first failed before implementation. It
covers stable IDs and disabled state across reload, rejection of a session's
socket alias, retained configuration while a target is offline, and explicit
removal. The Rust suite now contains 30 passing module tests; formatting and
Clippy with warnings denied passed. No e2e suite was added.

Target snapshots are private, bounded and atomically replaced under a separate
file lock. A local management command commits its state before notifying the
service. Registry replacement cancels only removed/disabled targets; unchanged
target objects and their active channels remain intact. Cloud state sync is
serialized and reads the latest grants and opaque target IDs under its lock.
Session/process restart detection and device-wide revocation are later slices.
