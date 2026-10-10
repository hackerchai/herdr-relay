import test from 'node:test';
import assert from 'node:assert/strict';
import { acceptFrame, channelLimits, initialFlow, quotas } from '../src/limits.ts';

test('persisted flow rejects forged credit, overflow and control floods without resetting progress', () => {
  const limits = channelLimits(quotas({}).stream, false);
  let a = initialFlow(0, limits),
    b = initialFlow(0, limits);
  const data = { kind: 1, offset: 0n, payload: new Uint8Array(16384) };
  for (let i = 0; i < 4; i++)
    ({ state: a, peer: b } = acceptFrame(a, b, { ...data, offset: BigInt(i * 16384) }, 0, limits));
  assert.equal(a.peak, 65536);
  assert.throws(() => acceptFrame(a, b, { ...data, offset: 65536n }, 0, limits));
  assert.throws(() =>
    acceptFrame(b, a, { kind: 2, offset: 65537n, payload: new Uint8Array() }, 1, limits),
  );
  const credited = acceptFrame(
    b,
    a,
    { kind: 2, offset: 16384n, payload: new Uint8Array() },
    10,
    limits,
  );
  b = credited.state;
  a = credited.peer;
  assert.equal(a.progress, 10);
  const duplicate = acceptFrame(
    b,
    a,
    { kind: 2, offset: 16384n, payload: new Uint8Array() },
    20,
    limits,
  );
  assert.equal(duplicate.forward, false);
  assert.equal(duplicate.peer.progress, 10);
  assert.throws(() =>
    acceptFrame(
      { ...a, sent: String(0xffffffffffffffffn - 1n) },
      { ...b, ack: String(0xffffffffffffffffn - 1n) },
      { ...data, offset: 0xffffffffffffffffn - 1n },
      30,
      limits,
    ),
  );
  assert.throws(() =>
    acceptFrame(a, b, { kind: 1, offset: BigInt(a.sent), payload: new Uint8Array() }, 30, limits),
  );
  assert.throws(() =>
    acceptFrame(
      initialFlow(0, limits),
      initialFlow(0, limits),
      { kind: 1, offset: 0n, payload: new Uint8Array(16385) },
      0,
      limits,
    ),
  );
  a = initialFlow(0, limits);
  b = initialFlow(0, limits);
  for (let i = 0; i < 300; i++)
    a = acceptFrame(a, b, { kind: 2, offset: 0n, payload: new Uint8Array() }, 0, limits).state;
  assert.throws(
    () => acceptFrame(a, b, { kind: 2, offset: 0n, payload: new Uint8Array() }, 0, limits),
    /RATE_LIMITED/,
  );
  // A wakeup reconstructs exactly these attachment values; it cannot refill
  // a spent budget without elapsed wall time or forget the offset/credit.
  const restored = structuredClone(a);
  assert.throws(() =>
    acceptFrame(restored, b, { kind: 2, offset: 0n, payload: new Uint8Array() }, 0, limits),
  );
  assert.doesNotThrow(() =>
    acceptFrame(restored, b, { kind: 2, offset: 0n, payload: new Uint8Array() }, 1000, limits),
  );
  assert.deepEqual(quotas({ MAX_DEVICE_ATTACH: '1', CHANNEL_BYTES_PER_SECOND: '16384' }), {
    attach: 1,
    short: 4,
    pending: 4,
    stream: { bytes_per_second: 16384, window: 262144 },
  });
  for (const env of [
    { MAX_DEVICE_ATTACH: '65' },
    { MAX_PENDING_CHANNELS: '33' },
    { MAX_DEVICE_SHORT: '0' },
    { CHANNEL_BYTES_PER_SECOND: '4194305' },
    { MAX_DEVICE_ATTACH: '01' },
  ])
    assert.throws(() => quotas(env), /RELAY_UNCONFIGURED/);
});
