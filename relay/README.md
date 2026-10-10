# Development Relay

This development configuration supports accountless pairing and persistent native
targets. The public service runs at `https://herdr-relay.hackerchai.com`; see the
[self-hosting guide](../docs/self-hosting.md) to deploy your own instance and
[operations guide](../docs/operations.md) for quotas. DeviceRelay routes encrypted channels; a separate Room DO holds each
target's exclusive lease.

```sh
npm ci
npm run check
npm test
npm run dev
```

The development origin is `https://relay.local.invalid`; Wrangler listens only
on loopback. Initialize two profiles with the Rust CLI, then run the publisher:

```sh
target/debug/herdr-relay --profile /absolute/profile-a serve \
  --origin https://relay.local.invalid --relay \
  --relay-local-url http://127.0.0.1:8787
```

Once `Relay online` appears, from another terminal:

```sh
target/debug/herdr-relay --profile /absolute/profile-a pair create |
  target/debug/herdr-relay --profile /absolute/profile-b pair accept \
    --relay-local-url http://127.0.0.1:8787
```

The Rust commands above run from the repository root. Both sides establish
inner mutual TLS; the loopback override is an explicit development transport.
Production uses HTTPS/WSS with normal hostname validation and no override.

For a controlled deployment, authenticate Wrangler, select the intended account
and a distinct Worker name, and set `RELAY_ORIGIN` to that Worker's real HTTPS
origin before deploying. Credentials belong in the environment or Wrangler's
credential store. Never add them to `wrangler.jsonc`. Validate the bundle with
`npm run deploy -- --dry-run`; then deploy with `npm run deploy` and repeat the
two-profile workflow without `--relay-local-url`. Set `ADMISSION_ENABLED` to
`false` and redeploy to reject new admission requests.

The checked-in development configuration is not a published default service.
See [wire details](../protocol/access-v1.md) and
[local validation](../docs/validation/2026-10-10-cloudflare-pairing.md).

To publish native targets, use `share add --name Work --session work` (or an
explicit `--socket`) and then `serve --relay`. Use the root README's `targets` and `connect` commands on the paired
profile, adding the same loopback override during local development. The
`v2` creates the SQLite Room DO; `v3` adds the single global Admission DO.
Keep all migration entries and bindings. See [operations](../docs/operations.md)
for coordinated global limits, signed aggregate status and finite retention.
