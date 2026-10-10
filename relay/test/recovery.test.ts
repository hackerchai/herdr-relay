import test from 'node:test';
import assert from 'node:assert/strict';
import { restoredChannelValid } from '../src/recovery.ts';
import { initialFlow } from '../src/limits.ts';
test('restored reservations retain consumed roles, exact credit and generation', () => {
  const row = {
    id: '1'.repeat(32),
    reservation: '2'.repeat(32),
    publisher: '3'.repeat(64),
    connector: '4'.repeat(64),
    boot: '5'.repeat(32),
    host_used: 1,
    client_used: 1,
    expires: 10000,
    purpose: 'attach',
    rate: 262144,
  };
  const role = (role: string) => ({
    kind: 'pair',
    role,
    channel: row.id,
    reservation: row.reservation,
    boot: row.boot,
    device: role === 'publisher' ? row.publisher : row.connector,
    purpose: 'attach',
    ready: true,
    messages: 1,
    limits: { bytes_per_second: row.rate },
    flow: initialFlow(0, { bytes_per_second: row.rate }),
  });
  const a = role('publisher'),
    b = role('connector');
  a.flow.sent = '65536';
  b.flow.ack = '1';
  const saved = structuredClone([a, b]);
  assert.equal(restoredChannelValid(row, saved, 1), true);
  assert.equal(restoredChannelValid(row, [a], 1), false);
  assert.equal(restoredChannelValid(row, [a, a], 1), false);
  assert.equal(restoredChannelValid(row, [a, { ...b, boot: '6'.repeat(32) }], 1), false);
  assert.equal(
    restoredChannelValid(row, [a, { ...b, flow: { ...b.flow, ack: '65537' } }], 1),
    false,
  );
  assert.equal(restoredChannelValid(row, [a, { ...b, flow: null }], 1), false);
  assert.equal(restoredChannelValid(row, saved, 10000), false);
  assert.equal(restoredChannelValid({ ...row, client_used: 0 }, [{ ...a, ready: false }], 1), true);
});
