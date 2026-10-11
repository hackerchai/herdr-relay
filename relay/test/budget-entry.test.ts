import test from 'node:test';
import assert from 'node:assert/strict';
import { platform } from './platform.ts';
import { digest } from '../src/protocol.ts';
const { default: worker } = await import('../src/index.ts');

const ORIGIN = 'https://relay.example';

async function signedRequest() {
  const path = '/v1/devices/' + 'a'.repeat(64) + '/status';
  const proof = {
    challenge: {
      version: 1,
      origin: ORIGIN,
      method: 'POST',
      path,
      device_id: 'b'.repeat(64),
      public_key: 'c'.repeat(64),
      nonce: 'd'.repeat(32),
      expires_at: '1800000060',
      payload_sha256: await digest('{}'),
    },
    signature: 'e'.repeat(128),
  };
  return new Request(ORIGIN + path, {
    method: 'POST',
    body: '{}',
    headers: { 'X-Relay-Proof': JSON.stringify(proof) },
  });
}

test('the Worker reports the Durable Object calls each request made', async () => {
  const finished: unknown[][] = [];
  const env = {
    RELAY_ORIGIN: ORIGIN,
    ADMISSION_ENABLED: 'true',
    ADMISSION: {
      getByName: () => ({
        begin: async () => ({ id: 'lease' }),
        finish: async (...args: unknown[]) => {
          finished.push(args);
        },
      }),
    },
    AUTH: { getByName: () => ({ consume: async () => 'b'.repeat(64) }) },
    DEVICES: { getByName: () => ({ fetch: async () => Response.json({}) }) },
    ROOMS: { getByName: () => assert.fail('not used') },
  };
  const ctx = platform();
  const response = await worker.fetch(await signedRequest(), env as never, ctx as never);
  await ctx.drain();
  assert.equal(response.status, 200);
  // AccessGate consumed the proof and DeviceRelay served the request.
  assert.deepEqual(finished, [['lease', 200, 2]]);
});

test('a spent monthly budget is remembered so later requests skip the Durable Object', async (t) => {
  let now = Date.UTC(2026, 9, 31, 23, 0);
  t.mock.method(Date, 'now', () => now);
  let begun = 0,
    budget = true;
  const env = {
    RELAY_ORIGIN: ORIGIN,
    ADMISSION_ENABLED: 'true',
    ADMISSION: {
      getByName: () => ({
        begin: async () => {
          begun++;
          return { code: 'GLOBAL_QUOTA_EXCEEDED', retry_after: 3600, budget };
        },
        finish: async () => {},
      }),
    },
    AUTH: { getByName: () => assert.fail('not used') },
    DEVICES: { getByName: () => assert.fail('not used') },
    ROOMS: { getByName: () => assert.fail('not used') },
  };
  const send = async () => {
    const ctx = platform();
    const response = await worker.fetch(await signedRequest(), env as never, ctx as never);
    await ctx.drain();
    assert.equal(response.status, 429);
    assert.equal((await response.json()).code, 'GLOBAL_QUOTA_EXCEEDED');
    return response;
  };
  assert.equal((await send()).headers.get('Retry-After'), '3600');
  const cached = await send();
  assert.equal(begun, 1);
  // Waiting hosts keep the full wait instead of retrying every minute.
  assert.equal(cached.headers.get('Retry-After'), '3600');
  // Each isolate asks again after a minute, so a raised budget or a new month
  // takes effect promptly.
  now += 60000;
  budget = false;
  await send();
  assert.equal(begun, 2);
  // A short-term quota refusal is not remembered.
  await send();
  assert.equal(begun, 3);
});
