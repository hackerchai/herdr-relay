import test from 'node:test';
import assert from 'node:assert/strict';
import { platform, TestSocket } from './platform.ts';
import { AdmissionLedger } from '../src/admission-ledger.ts';
const { DeviceRelay } = await import('../src/device-relay.ts');

test('publisher survives a transient renewal outage and schedules infrequent, bounded maintenance', async (t) => {
  let now = 0,
    unavailable = true;
  t.mock.method(Date, 'now', () => now);
  const ledger = new AdmissionLedger(platform().storage, { GLOBAL_PUBLISHERS: '1' }, () => now);
  const device = 'a'.repeat(64),
    boot = 'b'.repeat(32);
  const lease = ledger.hold('publisher', 'publisher:' + device, boot, 600000);
  const socket = new TestSocket({
    kind: 'control',
    device,
    boot,
    ready: false,
    expires: lease.expires,
  });
  const ctx = platform([socket]);
  const env = {
    ADMISSION_ENABLED: 'true',
    ADMISSION: {
      getByName: () => ({
        async renew(id: string, owner: string) {
          if (unavailable) {
            // A state-sync event can complete while the renewal RPC is pending.
            socket.serializeAttachment({ ...socket.attachment, ready: true });
            throw new Error('temporary RPC outage');
          }
          return ledger.renew(id, owner);
        },
        async release(id: string, owner: string) {
          ledger.release(id, owner);
        },
      }),
    },
  };
  let relay = new DeviceRelay(ctx, env);
  now = 300000;
  await ctx.fire(relay);
  assert.equal(socket.readyState, WebSocket.OPEN, 'an existing lease tolerates a failed RPC');
  assert.equal(socket.attachment.expires, 600000, 'failure never invents an extension');
  assert.equal(socket.attachment.ready, true, 'a renewal failure preserves concurrent state sync');
  assert.equal(await ctx.storage.getAlarm(), 330000);
  unavailable = false;
  now = 330000;
  relay = new DeviceRelay(ctx, env);
  await ctx.fire(relay);
  assert.equal(socket.attachment.expires, 930000);
  assert.equal(await ctx.storage.getAlarm(), 630000);
  now = 340000;
  await ctx.fire(relay);
  assert.equal(socket.attachment.expires, 930000);
  assert.equal(await ctx.storage.getAlarm(), 630000);
  unavailable = true;
  now = 930000;
  await ctx.fire(relay);
  assert.equal(socket.readyState, WebSocket.CLOSED, 'an expired lease fails closed');
});
