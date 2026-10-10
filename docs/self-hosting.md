# Self-host Herdr Relay

This runs the same Worker, device proof, pinned end-to-end TLS and native Herdr
protocol as the production hosted endpoint. A Cloudflare account belongs to the
operator; connecting devices need no Cloudflare account or deployment credential.
Worker source and these instructions are included in the public crate source
archive and the public GitHub repository. The hosted deployment runs on Workers
Paid; its production quotas bound usage beyond the plan's base. Self-hosters must
set their own cost policy; application quotas are not a provider spending cap.

## Deploy from a fresh account configuration

Install Node 24 and use the repository's pinned `package-lock.json`. Choose an
unused Worker name and your own Workers subdomain. Set up that subdomain in your
Cloudflare account first. The actual URL must match `RELAY_ORIGIN` exactly.

```sh
cd relay
npm ci
npx wrangler login
node scripts/configure.mjs --name my-herdr-relay \
  --origin https://my-herdr-relay.YOUR-SUBDOMAIN.workers.dev
```

The generator creates `.wrangler/selfhost.json` with private permissions and
refuses to overwrite an existing file. It sets `ADMISSION_ENABLED` to `false`.
Review it and set that value to `true` before allowing connections. For an
isolated acceptance deployment, add `--test-devices ID_A,ID_B` using the two
`identity show` outputs; that allowlist is a test restriction, not a device grant.
Omit it for accountless public admission only after setting an operational budget
and validating the global admission controls. Do not publish development keys.

```sh
npm run check
npm test
npx wrangler deploy --config .wrangler/selfhost.json
```

The config includes these SQLite Durable Object bindings and ordered migrations:

| Binding | Class | Migration |
| --- | --- | --- |
| AUTH | AccessGate | v1 |
| DEVICES | DeviceRelay | v1 |
| ROOMS | Room | v2 |
| ADMISSION | Admission | v3 |

Keep class names, namespace bindings and migration history stable on upgrades.
Worker names identify separate deployments. Renaming one is not an in-place
upgrade. See Cloudflare's [SQLite bindings and migrations](https://developers.cloudflare.com/durable-objects/reference/durable-objects-migrations/).

For CI, manage `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID` in your own secret
store. Use the account permissions needed to create/deploy Workers and Durable
Objects. Wrangler OAuth login is also supported. A Global API Key requires
`CLOUDFLARE_API_KEY` and `CLOUDFLARE_EMAIL`; it is not a Bearer API Token. Do not
put any of these credentials in vars, the source tree, CLI profiles or reports.
This Relay needs no application-wide bearer secret: devices sign fresh challenges.
Cloudflare's [authorization reference](https://developers.cloudflare.com/workers/authorization/workers/)
explains deploy/create permission scopes.

For a custom domain, configure a Cloudflare Worker custom-domain route and set
`RELAY_ORIGIN` to that exact HTTPS origin before pairing. Keep normal domain
certificate verification enabled. A wrong domain, HTTP origin, redirect or
untrusted certificate is not accepted by the production CLI. The explicit
loopback development option is for local Workers testing only.

## Pair and operate from empty profiles

Install verified native CLI candidates and official Herdr 0.9.3 on both devices.
Start the desired Herdr session yourself. On publisher A:

```sh
herdr-relay identity init
herdr-relay share add --name Work --session work
herdr-relay serve --relay --origin https://my-herdr-relay.YOUR-SUBDOMAIN.workers.dev
```

In another A terminal, run `herdr-relay pair create`. Treat its single-line output
as a secret and transfer it privately to B. On B:

```sh
herdr-relay identity init
herdr-relay pair accept
# Paste the invitation on stdin and press Enter.
herdr-relay targets --origin https://my-herdr-relay.YOUR-SUBDOMAIN.workers.dev --host A_DEVICE_ID
herdr-relay connect --origin https://my-herdr-relay.YOUR-SUBDOMAIN.workers.dev \
  --host A_DEVICE_ID --target TARGET_ID --herdr /absolute/path/to/herdr
```

A daemon can issue invitations while all Herdr sessions are offline. Pairing gives
B access to A's explicitly shared targets, not reverse access. Use native detach
(default Ctrl-B then Q) to exit; Herdr remains alive. A second client for the same
instance receives `TARGET_BUSY` and cannot displace the existing client.

Revoke from A with `herdr-relay peers revoke B_DEVICE_ID --origin ORIGIN`.
Active B connections end and new access fails. The reverse independent grant, if
present, remains separate. User service installation is described in [install](install.md).

## Change origin explicitly

A profile has one trust scope. Merely passing another `--origin` fails with
`PROFILE_MISMATCH`; it never copies authorizations to a different Relay.
Stop the profile's foreground daemon or user service and its connect commands:

```sh
herdr-relay service stop                 # if a user service was installed
herdr-relay relay switch --from https://old.example --to https://new.example
herdr-relay service install --origin https://new.example  # if using user services
herdr-relay service start
```

Alternatively start `serve --relay --origin https://new.example`. Repeat
`--start-at-login` during service installation if desired. The explicit switch
atomically clears incoming/outgoing grants, invitation receipts and pending
pairing requests. Identity keys and shared target definitions are preserved.
Both devices must pair again. Returning to the old origin does not revive old
grants or invitations. A running daemon prevents switching with `SERVICE_BUSY`.
A previously installed service still records its old origin until reinstalled;
trying to start it early fails closed. Back up the complete private profile before
operational migration; restoring an old snapshot can restore old grants, so
revoke those grants again before publishing a restored profile.

## Resource settings and diagnosis

Optional string-valued vars adjust these bounds:

| Var | Default | Allowed |
| --- | --- | --- |
| MAX_DEVICE_ATTACH | 8 | 1–64 across different targets |
| MAX_DEVICE_SHORT | 4 | 1–32 |
| MAX_PENDING_CHANNELS | 4 | 1–32 |
| CHANNEL_BYTES_PER_SECOND | 1048576 | 16384–4194304 per direction |
| CHANNEL_WINDOW_BYTES | 262144 | 65536–1048576 |

The rate above 262144 and the window apply only when both endpoints of a
channel run 0.2.4 or newer; older endpoints keep 256 KiB/s and the 64 KiB
window (see [negotiated capabilities](../protocol/capabilities.md)). Every
target still permits one active attach. No configuration changes the frame
bound or the 30-second stalled-progress timeout.

### CLI environment

`HERDR_RELAY_ORIGIN`, `HERDR_RELAY_PROFILE`, `HERDR_RELAY_HERDR` and
`HERDR_RELAY_UPDATE_REPOSITORY` set the matching options (including plain
`herdr-relay`); explicit flags win. `HERDR_RELAY_DIRECT=off` or
`HERDR_RELAY_DIRECT_PORT` configure the direct LAN listener of a host started
from that environment. `HERDR_RELAY_TIMING=1` prints connection phase timings
to stderr (or appends them to the file it names).
`ADMISSION_ENABLED=false` rejects new work with `RELAY_BUSY` and stops publisher
and channel lease renewal. Existing connections close on their next event or
maintenance alarm (normally within 30 seconds for active channels and five
minutes for idle publishers; forwarding also checks expiry).
Signed operator status remains available subject to entry quotas. This switch
alone is not a hard account-wide spending cap. See [operations](operations.md)
for global quotas, finite metadata retention, monitoring and emergency shutdown.

| Result | Operator/client action |
| --- | --- |
| Wrangler authentication/permission error | Check the deployment account, token type and scopes; do not send its value to Relay. |
| RELAY_UNCONFIGURED | Check canonical origin, all bindings, admission boolean and quota ranges. |
| INCOMPATIBLE_PROTOCOL | Use compatible CLI/Worker versions; do not bypass the version check. |
| ACCESS_INVALID / ACCESS_REQUIRED | Device proof is invalid/missing; old nonce or another origin cannot be reused. |
| NOT_AUTHORIZED | Pair again or ask the publisher to inspect the directional grant. |
| TARGET_BUSY | Wait for the current client to detach; no preemption. |
| QUOTA_EXCEEDED | Release this device's unused channels or adjust within bounds. |
| GLOBAL_QUOTA_EXCEEDED | Global capacity, requests or data-channel time is exhausted. Publishers and reconnecting clients wait automatically using Retry-After plus jitter. Host presence has no daily uptime cutoff. |
| RATE_LIMITED / RELAY_BUSY | Respect Retry-After and the CLI backoff. |
| CLOCK_SKEW | Enable automatic time synchronization. Small offsets are tolerated; larger offsets retry without changing the Relay's nonce expiry. |
| INTERNAL_ERROR | Inspect sanitized Worker error metadata and binding/migration health. |

Do not log payload bodies, proof/ticket headers, pairing invitations or terminal
content. Public responses expose stable codes, not exception stacks. Durable
Object status counters are restricted to the signed publisher and contain bounded
resource metadata. Cloudflare account diagnostics remain operator-only.

## Upgrade and recovery

Record the source/build version, config, deployed Worker version ID and private
endpoint profile backups. Validate in a separate controlled Worker before
redeploying the same name with unchanged bindings and migration history. Expect
WebSocket interruption on deployment; endpoints re-prove identity, establish new
TLS/channels and reconnect without replaying offline input.

Worker rollback does not roll back Durable Object storage or endpoint grants.
Do not delete/recreate namespaces to clear a transient problem, apply old SQL
snapshots blindly, or deploy an older binary that cannot read current state.
If a migration prevents rollback, repair forward with a compatible version.
For suspected inconsistent channel state, stop publishers and let bounded leases
expire, then restart; fail-closed restoration rejects mismatched roles/generations.
Authoritative grants and target definitions remain on the publisher. Loss of an
endpoint's identity private key requires a new identity and new pairing, not
silent replacement of an existing identity. Recovery must never resurrect a
revoked grant from a stale cloud mirror.
