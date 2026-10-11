import test from 'node:test';
import assert from 'node:assert/strict';
import { platform, TestSocket } from './platform.ts';
import { initialFlow } from '../src/limits.ts';
const { DeviceRelay } = await import('../src/device-relay.ts');

const publisher = 'a'.repeat(64),
  connector = 'c'.repeat(64),
  boot = 'b'.repeat(32),
  channel = 'd'.repeat(32),
  reservation = 'e'.repeat(32),
  target = 'f'.repeat(32);

function frame(kind: number, offset: bigint, size: number): ArrayBuffer {
  const buffer = new ArrayBuffer(32 + size),
    view = new DataView(buffer);
  view.setUint8(0, 1);
  view.setUint8(1, kind);
  new Uint8Array(buffer, 4, 16).fill(0xdd);
  view.setBigUint64(20, offset);
  view.setUint32(28, size);
  return buffer;
}

test('a DeviceRelay reports its incoming messages and calls with each Admission call, across hibernation', async (t) => {
  const now = 1_000_000;
  t.mock.method(Date, 'now', () => now);
  const limits = { bytes_per_second: 1048576, window: 262144 };
  const control = new TestSocket({
    kind: 'control',
    device: publisher,
    boot,
    ready: true,
    expires: now + 1e9,
  });
  const pair = (role: string, device: string) =>
    new TestSocket({
      kind: 'pair',
      purpose: 'attach',
      device,
      boot,
      channel,
      reservation,
      role,
      ready: true,
      expires: now + 30000,
      flow: initialFlow(now, limits),
      limits,
      messages: 0,
    });
  const host = pair('publisher', publisher),
    client = pair('connector', connector);
  const sockets = [control, host, client];
  const ctx = platform(sockets);
  ctx.getWebSockets = (tag?: string) =>
    sockets.filter(
      (s) =>
        !tag ||
        (tag === 'control'
          ? s.attachment.kind === 'control'
          : 'channel:' + s.attachment.channel === tag),
    );
  const reports: { method: string; usage: unknown }[] = [];
  const env = {
    ADMISSION_ENABLED: 'true',
    ADMISSION: {
      getByName: () => ({
        async renew(_id: string, _owner: string, usage: unknown) {
          reports.push({ method: 'renew', usage });
          return { expires: now + 1e9 };
        },
        async release(_id: string, _owner: string, usage: unknown) {
          reports.push({ method: 'release', usage });
        },
      }),
    },
    ROOMS: { getByName: () => ({ renew: async () => now + 1e9, release: async () => {} }) },
  };
  new DeviceRelay(ctx as never, env as never);
  ctx.storage.sql.exec(
    "INSERT INTO channels (id,reservation,publisher,connector,boot,host_ticket,client_ticket,host_used,client_used,expires,purpose,target,grant_version,authenticated,rate,window) VALUES (?,?,?,?,?,'h','c',1,1,?,'attach',?,1,1,?,?)",
    channel,
    reservation,
    publisher,
    connector,
    boot,
    now + 30000,
    target,
    limits.bytes_per_second,
    limits.window,
  );
  ctx.storage.sql.exec('INSERT INTO grants VALUES (?,1)', connector);
  ctx.storage.sql.exec('INSERT INTO targets VALUES (?)', target);
  for (const s of sockets) s.readyState = WebSocket.OPEN;
  let relay = new DeviceRelay(ctx as never, env as never);
  let offset = 0n;
  const exchange = async () => {
    await relay.webSocketMessage(host as unknown as WebSocket, frame(1, offset, 8));
    offset += 8n;
    await relay.webSocketMessage(client as unknown as WebSocket, frame(2, offset, 0));
  };
  for (let i = 0; i < 3; i++) await exchange();
  relay = new DeviceRelay(ctx as never, env as never); // woke from hibernation
  await ctx.fire(relay);
  // Six frames; the alarm itself is one invocation.
  assert.deepEqual(reports, [{ method: 'renew', usage: { frames: 6, calls: 1 } }]);
  for (let i = 0; i < 2; i++) await exchange();
  relay.webSocketClose(host as unknown as WebSocket);
  await ctx.drain();
  // Only the frames since the last report; the earlier Room renewal is a call.
  assert.deepEqual(reports[1], { method: 'release', usage: { frames: 4, calls: 1 } });
});
