# Herdr compatibility

Herdr Relay does not restrict connections to a list of Herdr release numbers.
Stable releases, previews and custom builds can connect when they retain the
native client interface and share a compatible endpoint protocol with the host.
Herdr 0.9.3 and 0.9.4 have been tested in both client/server directions, including
terminal input, returned screen output and clean detach through Relay's local
byte-stream bridge.

Relay runs `herdr client` with `HERDR_CLIENT_SOCKET_PATH` pointing at a private
local socket. It forwards the native bytes unchanged. The Herdr endpoints
negotiate their protocol generation and codecs, so a patch, minor or major
release number alone does not cause Relay to reject a connection. The
[upstream handshake](https://github.com/herdrdev/herdr/blob/v0.9.4/src/client/handshake.rs)
defines that negotiation.

The initial `--version` check only verifies that the configured command runs
successfully and identifies itself as `herdr <version>`. It accepts semantic
versions with preview/custom suffixes and build metadata. A missing command,
invalid response, excessive output or a hung process still fails promptly.

Very old versions without this client interface, or future versions that remove
the common endpoint protocol, may still be incompatible. Herdr reports native
handshake failures in the terminal; Relay does not translate protocols or force
a rejected handshake to succeed. Use compatible Herdr builds on both computers
if that occurs. Set `--herdr /absolute/path/to/herdr` or
`herdr-relay config set herdr /absolute/path/to/herdr` to choose another client.

The optional [bounded 0.9.3 server build](../patches/herdr/README.md) addresses
that server's control queue. It is not a required Relay client version.
