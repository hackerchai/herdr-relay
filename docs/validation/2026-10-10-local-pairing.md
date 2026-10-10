# Local pairing acceptance

2026-10-10, macOS arm64. Covers issue #3 using two isolated profiles and a
foreground publisher service. Herdr was not running for this workflow.

Manual results:

- `serve --origin https://local-pair.invalid` opened an owned, mode-0600 Unix
  socket in the private profile. `pair create` returned a ten-minute invitation.
- B imported through stdin and completed inner TLS authentication, context
  binding and redemption. A listed B under allowed devices; B listed A under
  accessible hosts. Neither acquired the opposite grant.
- C importing the same invitation received `PAIR_INVALID` and exit 1.
- An interactive PTY paste disabled echo and did not print the invitation.
  Both successful input and SIGINT cancellation restored the exact original
  termios state. Cancellation exited 130.
- After SIGTERM stopped A, importing returned `HOST_OFFLINE` and exit 1.
  Restarting A preserved its grant. B re-imported the original invitation and
  recovered grant version 1 using its persisted request ID.
- The stored state was mode 0600 and contained only the secret digest, not
  the invitation's raw secret. Service diagnostics emitted only error codes.

Necessary module checks also passed: concurrent different-device redemption
allows one winner; only its original identity/request/secret can recover the
receipt within 24 hours; expired, revoked and wrong-secret requests fail;
an actual filesystem write denial commits no grant and leaves the invitation
usable after the failure is repaired; forged handshake signatures fail even
at the unpaired entry point. The unpaired handler accepts exactly one bounded
redemption message and has no target lookup or native-socket dispatch.

Storage is one bounded private JSON snapshot, with an independent process
lock, fsync before atomic replacement and a directory fsync before reporting
success. Limits are 4 live invitations, 128 retained invitation results,
64 grants per direction, 64 pending requests and 1 MiB of persisted state.
Only a digest is stored at the publisher; the connector stores the request ID
and re-imports the original secret when retrying. Invitation wire format is
`hrp1:` followed by hex-encoded strict JSON; it remains a development format.

Formatting, all 23 module tests and Clippy with warnings denied passed. No e2e
test suite was added. This local transport establishes no Cloudflare or remote
network validation; that is tracked by issue #4.
