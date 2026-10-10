import test from 'node:test';
import assert from 'node:assert/strict';
import { platform } from './platform.ts';
import { digest } from '../src/protocol.ts';
const { default: worker } = await import('../src/index.ts');

test('authentication infrastructure failures remain retryable without accepting invalid proofs', async () => {
  const proof = {
    challenge: {
      version: 1,
      origin: 'https://relay.example',
      method: 'POST',
      path: '/v1/devices/' + 'a'.repeat(64) + '/status',
      device_id: 'b'.repeat(64),
      public_key: 'c'.repeat(64),
      nonce: 'd'.repeat(32),
      expires_at: '1800000060',
      payload_sha256: await digest('{}'),
    },
    signature: 'e'.repeat(128),
  };
  for (const [failure, status, code] of [
    ['temporary DO storage failure', 503, 'RELAY_BUSY'],
    ['ACCESS_INVALID', 401, 'ACCESS_INVALID'],
    ['ACCESS_REPLAYED', 401, 'ACCESS_INVALID'],
  ] as const) {
    const env = {
      RELAY_ORIGIN: 'https://relay.example',
      ADMISSION_ENABLED: 'true',
      ADMISSION: {
        getByName: () => ({ begin: async () => ({ id: 'request' }), finish: async () => {} }),
      },
      AUTH: {
        getByName: () => ({
          consume: async () => {
            throw new Error(failure);
          },
        }),
      },
      DEVICES: {
        getByName: () => {
          assert.fail('failed authentication cannot forward a request');
        },
      },
      ROOMS: {
        getByName: () => {
          assert.fail('failed authentication cannot reserve a channel');
        },
      },
    };
    const ctx = platform();
    const response = await worker.fetch(
      new Request('https://relay.example' + proof.challenge.path, {
        method: 'POST',
        body: '{}',
        headers: { 'X-Relay-Proof': JSON.stringify(proof) },
      }),
      env,
      ctx,
    );
    await ctx.drain();
    assert.equal(response.status, status, failure);
    assert.equal((await response.json()).code, code);
    if (status === 503) assert.ok(Number(response.headers.get('Retry-After')) >= 1);
  }
});
