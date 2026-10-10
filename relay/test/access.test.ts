import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { canonicalAccess, verifyAccess, strictJson } from '../src/protocol.ts';

const fixture = JSON.parse(
  readFileSync(new URL('../../protocol/fixtures/access-v1.json', import.meta.url), 'utf8'),
);
test('shared access fixture verifies and cannot cross origin, operation, identity or payload', async () => {
  assert.equal(canonicalAccess(fixture.challenge), fixture.canonical);
  assert.equal(await verifyAccess(fixture, 'https://relay.example', 1800000000), true);
  for (const patch of [
    { origin: 'https://different.example' },
    { method: 'GET', path: '/v1/devices/' + '11'.repeat(32) + '/control' },
    { path: '/v1/devices/' + '12'.repeat(32) + '/pair' },
    { device_id: '00'.repeat(32) },
    { nonce: '33'.repeat(16) },
    { expires_at: '1800000059' },
    { payload_sha256: '00'.repeat(32) },
  ]) {
    const changed = structuredClone(fixture);
    Object.assign(changed.challenge, patch);
    assert.equal(
      await verifyAccess(changed, changed.challenge.origin, 1800000000),
      false,
      JSON.stringify(patch),
    );
  }
  assert.equal(await verifyAccess(fixture, 'https://other.example', 1800000000), false);
  assert.equal(await verifyAccess(fixture, 'https://relay.example', 1800000060), false);
  const forged = { ...fixture, signature: '00'.repeat(64) };
  assert.equal(await verifyAccess(forged, 'https://relay.example', 1800000000), false);
});
test('strict bounded control JSON rejects duplicate keys and noncanonical encodings', () => {
  assert.deepEqual(strictJson('{"version":1}'), { version: 1 });
  for (const input of [
    '{"version":1,"version":1}',
    '{"version":1.0}',
    '{"version":1} ',
    'x'.repeat(8193),
    '{bad',
  ]) {
    assert.throws(() => strictJson(input));
  }
});
