# Ticket #7: exclusivity and symmetric device roles

2026-10-10, macOS arm64, two real Herdr 0.9.3 sessions and three isolated
profiles. Live Worker version `76dffd99-3454-4bd4-95d7-c0271a38233d` at the
same controlled development origin as earlier records.

Two signed `access` requests from B and C raced for A's Work target. Exactly
one returned a reservation; the other returned HTTP 409 `TARGET_BUSY`.
The winning request did not finish its channel handshake. Its pending
reservation expired/closed, permitting later access without opening a native
socket for that unfinished request.

A initially lacked B's independent grant and could not query B:
`NOT_AUTHORIZED`. B then issued an invitation to A. After A redeemed it, A's
existing service continued publishing Work while its foreground client
connected to B's Personal. B simultaneously published Personal and connected
to A's Work. Unique `A_TO_B_979191927009708` and `B_TO_A_979191927009708`
commands reached the intended panes.

While B held Work, both B's second connect and C's connect returned
`TARGET_BUSY`. B detached normally; C then connected and printed
`C_AFTER_RELEASE_979191927009708`. All normal exits returned 0 and restored
exact terminal flags. No operation took over an occupied instance.

A temporary local Worker exercised the actual Room class and SQLite storage
through RPC, not a mocked storage API. Concurrent reserves produced one
winner. Releasing an old reservation after replacement could not free the
new one; old renewal failed and current renewal succeeded. Expired leases
could not renew, a new lease could replace them, and late release of the
expired generation could not free its replacement. All eight checks passed.
The probe was kept outside tracked source; no e2e suite was added.

DeviceRelay also rejects another attach row for the same opaque target before
and after the Room RPC. That local check avoids admitting overlapping routes
if independent objects disagree about a lease deadline. Native inode locking
independently enforces the same rule on the host. Role/device/ticket replay
negatives and wrong inner TLS contexts are covered by earlier records/tests.

Device-wide revocation and its races were subsequently implemented and measured
in [the #8 acceptance record](2026-10-10-revocation.md).
