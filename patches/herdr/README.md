# Bounded Herdr 0.9.3 server

This is a custom compatibility build based on upstream commit
`7b116c05bfda646af39d2524c54e70c751f57ee8`. It is not an upstream Herdr release.
The patch and modified upstream material are Apache-2.0 licensed; see
`LICENSE-APACHE`. The rest of Herdr Relay uses the root MIT license.

The patch limits each client's pending reliable-control lane to 256 messages
and 67,108,880 bytes of allocated vector capacity. Capacity is returned when the
writer takes a frame. One additional frame can be in the writer; render slots
keep their existing upstream bounds. This is a queue allocation bound, not a
hard limit on the entire Herdr process, terminal history or unrelated panes.

Overflow rejects the new frame and terminates that client. It releases all
queued lanes, wakes condition-variable waiters and, on macOS/Linux, shuts down
both directions of the native socket so blocked I/O exits. Other clients retain
their own independent queues. The endpoint wire format and codecs are unchanged.
Three regression tests cover message overflow/isolation, byte capacity with
drain/refill, and actual blocked native read/write cancellation. The 32 tests in
upstream `server::client_transport::tests` pass on the local macOS arm64 build.

Build with Rust 1.99 and Zig 0.16.0:

```sh
scripts/build-herdr.sh prepare
scripts/build-herdr.sh test
scripts/build-herdr.sh build
```

The script verifies the pinned source archive and patch hash. Builds remain under
`target/upstream/herdr/target`; it never installs a binary, starts a server, or
replaces an existing session. Run the resulting `herdr` explicitly for new server
sessions and use `herdr-relay connect --herdr /path/to/this/herdr` for its client.
The version is `0.9.3-custom.relay-control-bound-1`. Existing official 0.9.3 clients
remain wire-compatible. Existing servers must be restarted with this binary to
receive the fix; attaching Relay to an unmodified server cannot impose its
missing internal queue bound.

GitHub release automation builds separate `herdr-bounded-0.9.3-TARGET.tar.gz`
assets for the four supported Unix targets, alongside the Relay CLI packages.
They contain the patch, license, provenance and checksums. This repository is
private; public crates.io source includes this patch and build script so users
can build it without access to private release assets.
