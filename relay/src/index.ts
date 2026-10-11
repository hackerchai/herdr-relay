import { Room } from './room.ts';
import { AccessGate } from './access-gate.ts';
import { DeviceRelay, PREPARED_JOIN, features, prepareJoin } from './device-relay.ts';
import { quotas } from './limits.ts';
import { Admission, ADMISSION_OBJECT, admitted } from './admission.ts';
import { globalQuotas, type AdmissionEnv } from './admission-ledger.ts';
import {
  type Proof,
  PUBLIC_ORIGIN,
  ProtocolError,
  boundedBody,
  bytes,
  challenge,
  digest,
  failure,
  isHex,
  originValid,
  record,
  route,
  strictJson,
} from './protocol.ts';
export { AccessGate, DeviceRelay, Room, Admission };
export interface Env extends AdmissionEnv {
  RELAY_ORIGIN: string;
  ADMISSION_ENABLED: string;
  /** Optional allowlist for a controlled development deployment, never a device grant. */
  TEST_DEVICE_IDS?: string;
  OPERATOR_DEVICE_IDS?: string;
  METADATA_RETENTION_HOURS?: string;
  MAX_DEVICE_ATTACH?: string;
  MAX_DEVICE_SHORT?: string;
  MAX_PENDING_CHANNELS?: string;
  CHANNEL_BYTES_PER_SECOND?: string;
  /** Optional shared secret a self-hosted relay requires as `Authorization: Bearer`. */
  RELAY_ACCESS_TOKEN?: string;
  AUTH: DurableObjectNamespace<AccessGate>;
  DEVICES: DurableObjectNamespace<DeviceRelay>;
  ROOMS: DurableObjectNamespace<Room>;
  ADMISSION: DurableObjectNamespace<Admission>;
}
/**
 * When this isolate last learned the monthly budget is spent, it refuses
 * requests itself until then, so a stopped relay costs no Durable Object calls.
 */
let stoppedUntil = 0;
/** When the budget refusal told clients to come back. */
let resumeAt = 0;
const BUDGET_RECHECK_MS = 60000;

export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    if (request.method === 'GET' && new URL(request.url).pathname === '/pair') {
      return new Response(
        '<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>Pair Herdr Relay</title><main><h1>Connect to your Herdr host</h1><p>On the client computer, run <code>herdr-relay pair</code> and paste the complete link from your address bar.</p><p>This page does not redeem the invitation. Pairing links expire after ten minutes and can grant access once.</p></main></html>',
        {
          headers: {
            'Content-Type': 'text/html; charset=utf-8',
            'Content-Security-Policy': "default-src 'none'; frame-ancestors 'none'",
            'Referrer-Policy': 'no-referrer',
            'Cache-Control': 'no-store',
          },
        },
      );
    }
    let lease: string | undefined,
      status = 200,
      // Durable Object calls besides begin/finish, reported for the monthly budget.
      calls = 0;
    try {
      if (
        !originValid(env.RELAY_ORIGIN) ||
        !['true', 'false'].includes(env.ADMISSION_ENABLED) ||
        [env.AUTH, env.DEVICES, env.ROOMS, env.ADMISSION].some(
          (binding) => !binding || typeof binding.getByName !== 'function',
        )
      )
        throw new ProtocolError('RELAY_UNCONFIGURED', 503);
      quotas(env);
      globalQuotas(env);
      if (
        env.METADATA_RETENTION_HOURS !== undefined &&
        (!/^[1-9][0-9]*$/.test(env.METADATA_RETENTION_HOURS) ||
          Number(env.METADATA_RETENTION_HOURS) > 168)
      )
        throw new ProtocolError('RELAY_UNCONFIGURED', 503);
      if (
        env.OPERATOR_DEVICE_IDS &&
        !env.OPERATOR_DEVICE_IDS.split(',').every((id) => isHex(id, 64))
      )
        throw new ProtocolError('RELAY_UNCONFIGURED', 503);
      // Checked before admission so an unauthorized caller never reaches a Durable Object.
      await authorize(request, env);
      const url = new URL(request.url);
      if (
        env.ADMISSION_ENABLED !== 'true' &&
        !['/v1/challenge', '/v1/operator/status'].includes(url.pathname)
      )
        throw new ProtocolError('RELAY_BUSY', 503);
      // All requests, including malformed/unsigned requests and newly minted
      // identities, pass the same durable quota BEFORE body reads or per-device DO creation.
      const now = Date.now();
      if (now < stoppedUntil)
        throw new ProtocolError(
          'GLOBAL_QUOTA_EXCEEDED',
          429,
          Math.max(1, Math.ceil((resumeAt - now) / 1000)),
        );
      const ip = await digest(request.headers.get('CF-Connecting-IP') ?? 'loopback-development');
      const decision = await env.ADMISSION.getByName(ADMISSION_OBJECT).begin(ip);
      if (decision.budget) {
        resumeAt = now + (decision.retry_after ?? 60) * 1000;
        stoppedUntil = Math.min(resumeAt, now + BUDGET_RECHECK_MS);
      }
      lease = admitted(decision).id;
      if (url.search) throw new ProtocolError();
      const body = await boundedBody(request);
      if (request.method === 'POST' && url.pathname === '/v1/challenge') {
        const value = record(strictJson(body), [
          'version',
          'origin',
          'method',
          'path',
          'device_id',
          'public_key',
          'payload_sha256',
        ]);
        if (!isHex(value.device_id, 64)) throw new ProtocolError();
        if (env.TEST_DEVICE_IDS && !env.TEST_DEVICE_IDS.split(',').includes(value.device_id))
          throw new ProtocolError('DEVELOPMENT_ONLY', 403);
        const checked = challenge({
          ...value,
          nonce: '0'.repeat(32),
          expires_at: String(Math.floor(Date.now() / 1000) + 60),
        });
        const operator =
          checked.path === '/v1/operator/status' &&
          env.OPERATOR_DEVICE_IDS?.split(',').includes(checked.device_id);
        if (checked.path === '/v1/operator/status' && !operator)
          throw new ProtocolError('NOT_AUTHORIZED', 403);
        if (env.ADMISSION_ENABLED !== 'true' && !operator)
          throw new ProtocolError('RELAY_BUSY', 503);
        if (
          checked.origin !== env.RELAY_ORIGIN ||
          (await digest(bytes('302a300506032b6570032100' + checked.public_key))) !==
            checked.device_id
        )
          throw new ProtocolError('ACCESS_INVALID', 401);
        let result;
        // The challenge's expiry alarm in AccessGate is a second invocation.
        calls += 2;
        try {
          result = await env.AUTH.getByName('device:' + value.device_id).issue(value);
        } catch (error) {
          if (error instanceof Error && error.message === 'RATE_LIMITED')
            throw new ProtocolError('RATE_LIMITED', 429);
          if (error instanceof Error && error.message === 'GLOBAL_QUOTA_EXCEEDED')
            throw new ProtocolError('GLOBAL_QUOTA_EXCEEDED', 429, 120);
          throw new ProtocolError('RELAY_BUSY', 503);
        }
        return Response.json(result, { headers: { 'Cache-Control': 'no-store' } });
      }
      const destination = route(request.method, url.pathname);
      const proofText = request.headers.get('X-Relay-Proof');
      if (!proofText) throw new ProtocolError('ACCESS_REQUIRED', 401);
      const value = record(strictJson(proofText), ['challenge', 'signature']);
      const proof: Proof = {
        challenge: challenge(value.challenge),
        signature: typeof value.signature === 'string' ? value.signature : '',
      };
      if (
        env.TEST_DEVICE_IDS &&
        !env.TEST_DEVICE_IDS.split(',').includes(proof.challenge.device_id)
      )
        throw new ProtocolError('DEVELOPMENT_ONLY', 403);
      let payload = body;
      if (destination.operation === 'join') payload = request.headers.get('X-Relay-Ticket') ?? '';
      if (destination.operation === 'control') payload = request.headers.get('X-Relay-Boot') ?? '';
      if (
        proof.challenge.method !== request.method ||
        proof.challenge.path !== url.pathname ||
        proof.challenge.payload_sha256 !== (await digest(payload))
      )
        throw new ProtocolError('ACCESS_INVALID', 401);
      let device: string;
      calls++;
      try {
        device = await env.AUTH.getByName('device:' + proof.challenge.device_id).consume(proof);
      } catch (error) {
        // RPC preserves the error message, not custom Error properties.
        // Storage/network failures must not masquerade as invalid credentials.
        if (error instanceof Error && ['ACCESS_INVALID', 'ACCESS_REPLAYED'].includes(error.message))
          throw new ProtocolError('ACCESS_INVALID', 401);
        throw new ProtocolError('RELAY_BUSY', 503, 5);
      }
      if (destination.operation === 'operator') {
        if (!env.OPERATOR_DEVICE_IDS?.split(',').includes(device))
          throw new ProtocolError('NOT_AUTHORIZED', 403);
        record(strictJson(body), []);
        calls++;
        return Response.json(await env.ADMISSION.getByName(ADMISSION_OBJECT).status(), {
          headers: { 'Cache-Control': 'no-store' },
        });
      }
      const headers = new Headers(request.headers);
      headers.set('X-Relay-Device', device);
      headers.set('X-Relay-Public-Key', proof.challenge.public_key);
      headers.delete('X-Relay-Proof');
      const forwarded = new Request(request.url, {
        method: request.method,
        headers,
        body: request.method === 'POST' ? body : undefined,
      });
      calls++;
      const response = await env.DEVICES.getByName(destination.publisher).fetch(forwarded);
      status = response.status;
      if (
        response.status === 200 &&
        ['access', 'pair'].includes(destination.operation) &&
        features(request).has(PREPARED_JOIN)
      ) {
        calls += 2; // AccessGate issues the join challenge and later expires it.
        return await withJoinChallenge(env, response, destination.publisher, proof.challenge);
      }
      return response;
    } catch (error) {
      const response = failure(error);
      status = response.status;
      return response;
    } finally {
      if (lease)
        ctx.waitUntil(
          env.ADMISSION.getByName(ADMISSION_OBJECT)
            .finish(lease, status, calls)
            .catch(() => {}),
        );
    }
  },
} satisfies ExportedHandler<Env>;

async function authorize(request: Request, env: Env): Promise<void> {
  const token = env.RELAY_ACCESS_TOKEN;
  if (token === undefined || env.RELAY_ORIGIN === PUBLIC_ORIGIN) return;
  if (!/^[\x21-\x7e]{16,512}$/.test(token)) throw new ProtocolError('RELAY_UNCONFIGURED', 503);
  const header = request.headers.get('Authorization') ?? '';
  if (!header.startsWith('Bearer ')) throw new ProtocolError('RELAY_AUTH_REQUIRED', 401);
  // Compare fixed-length digests so the comparison time reveals nothing about the token.
  const [given, expected] = await Promise.all([
    crypto.subtle.digest('SHA-256', new TextEncoder().encode(header.slice(7))),
    crypto.subtle.digest('SHA-256', new TextEncoder().encode(token)),
  ]);
  const a = new Uint8Array(given),
    b = new Uint8Array(expected);
  let difference = 0;
  for (let i = 0; i < a.length; i++) difference |= a[i] ^ b[i];
  if (difference !== 0) throw new ProtocolError('RELAY_AUTH_INVALID', 401);
}

/** Issue the connector's join challenge here, near the connector. */
async function withJoinChallenge(
  env: Env,
  response: Response,
  publisher: string,
  proof: Proof['challenge'],
): Promise<Response> {
  const opened = (await response.json()) as Record<string, unknown>;
  if (!isHex(opened.channel_id, 32) || !isHex(opened.ticket, 64))
    return Response.json(opened, { headers: { 'Cache-Control': 'no-store' } });
  const challenge = await prepareJoin(
    env,
    proof.device_id,
    proof.public_key,
    `/v1/devices/${publisher}/channels/${opened.channel_id}/connector`,
    opened.ticket,
  );
  return Response.json(challenge ? { ...opened, join_challenge: challenge } : opened, {
    headers: { 'Cache-Control': 'no-store' },
  });
}
