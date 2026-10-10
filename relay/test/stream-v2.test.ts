import test from 'node:test';
import assert from 'node:assert/strict';
import { acceptFrame, channelLimits, initialFlow, quotas } from '../src/limits.ts';

const credit = (offset: bigint) => ({ kind: 2, offset, payload: new Uint8Array() });
const data = (offset: bigint, size = 16384) => ({
  kind: 1,
  offset,
  payload: new Uint8Array(size),
});

test('self-hosted relays can raise the channel rate and window within hard bounds', () => {
  const raised = quotas({ CHANNEL_BYTES_PER_SECOND: '4194304', CHANNEL_WINDOW_BYTES: '1048576' });
  assert.deepEqual(raised.stream, { bytes_per_second: 4194304, window: 1048576 });
  assert.throws(() => quotas({ CHANNEL_BYTES_PER_SECOND: '4194305' }));
  assert.throws(() => quotas({ CHANNEL_WINDOW_BYTES: '65535' }));
  assert.throws(() => quotas({ CHANNEL_WINDOW_BYTES: '1048577' }));
  assert.ok(quotas({ MAX_DEVICE_ATTACH: '32', MAX_PENDING_CHANNELS: '16' }));
});

test('endpoints that did not negotiate stream v2 keep the legacy 64 KiB window and 256 KiB/s cap', () => {
  const stream = quotas({
    CHANNEL_BYTES_PER_SECOND: '1048576',
    CHANNEL_WINDOW_BYTES: '262144',
  }).stream;
  assert.deepEqual(channelLimits(stream, false), { bytes_per_second: 262144 });
  assert.deepEqual(channelLimits(stream, true), { bytes_per_second: 1048576, window: 262144 });
  const legacy = channelLimits(stream, false);
  let a = initialFlow(0, legacy),
    b = initialFlow(0, legacy);
  for (let i = 0; i < 4; i++)
    ({ state: a, peer: b } = acceptFrame(a, b, data(BigInt(i * 16384)), 0, legacy));
  assert.throws(() => acceptFrame(a, b, data(65536n), 0, legacy));
});

test('a negotiated window carries more data in flight than the legacy window', () => {
  const limits = channelLimits(quotas({ CHANNEL_WINDOW_BYTES: '262144' }).stream, true);
  let a = initialFlow(0, limits),
    b = initialFlow(0, limits);
  for (let i = 0; i < 16; i++)
    ({ state: a, peer: b } = acceptFrame(a, b, data(BigInt(i * 16384)), 0, limits));
  assert.equal(a.peak, 262144);
  assert.throws(() => acceptFrame(a, b, data(262144n), 0, limits));
});

test('credits use their own message budget once stream v2 is negotiated', () => {
  const stream = quotas({}).stream;
  for (const v2 of [false, true]) {
    const limits = channelLimits(stream, v2);
    // Exhaust the data message budget with tiny keystroke frames.
    let a = initialFlow(0, limits),
      b = initialFlow(0, limits);
    for (let i = 0; i < 300; i++) {
      ({ state: a, peer: b } = acceptFrame(a, b, data(BigInt(i), 1), 0, limits));
      ({ state: b, peer: a } = acceptFrame(b, a, credit(BigInt(i + 1)), 0, limits));
    }
    // b sent 300 credits; a sent 300 data frames. a now acknowledges b's data.
    b = { ...b, sent: '1' };
    const ack = () => acceptFrame(a, b, credit(1n), 0, limits);
    if (v2) assert.equal(ack().state.ack, '1');
    else assert.throws(ack);
  }
});

test('a restored stream v2 channel keeps its negotiated window and credit budget', async () => {
  const { restoredChannelValid } = await import('../src/recovery.ts');
  const limits = { bytes_per_second: 1048576, window: 262144 };
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
    rate: limits.bytes_per_second,
    window: limits.window,
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
    limits,
    flow: initialFlow(0, limits),
  });
  const a = role('publisher'),
    b = role('connector');
  a.flow.sent = '200000';
  a.flow.peak = 200000;
  assert.equal(restoredChannelValid(row, structuredClone([a, b]), 1), true);
  // The same state is impossible on a legacy channel.
  const legacy = { ...row, window: 0, rate: 262144 };
  const legacyRole = (r: ReturnType<typeof role>) => ({
    ...r,
    limits: { bytes_per_second: 262144 },
  });
  assert.equal(restoredChannelValid(legacy, [legacyRole(a), legacyRole(b)], 1), false);
  // A v2 attachment cannot be restored onto a legacy row or vice versa.
  assert.equal(restoredChannelValid({ ...row, window: 0 }, structuredClone([a, b]), 1), false);
});
