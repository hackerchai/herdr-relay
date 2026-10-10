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

function setup(t: { mock: { method: (o: object, k: string, f: () => number) => void } }) {
  let now = 1_000_000;
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
  const stub = {
    getByName: () => ({
      async release() {},
      async renew() {
        return { expires: now + 1e9 };
      },
    }),
  };
  const env = { ADMISSION_ENABLED: 'true', ADMISSION: stub, ROOMS: stub };
  new DeviceRelay(ctx, env);
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
  const relay = new DeviceRelay(ctx, env);
  let offset = 0n;
  const exchange = async () => {
    await relay.webSocketMessage(host as unknown as WebSocket, frame(1, offset, 8));
    offset += 8n;
    await relay.webSocketMessage(client as unknown as WebSocket, frame(2, offset, 0));
  };
  return {
    ctx,
    host,
    client,
    exchange,
    advance: (ms: number) => (now += ms),
    renew: (ms: number) =>
      ctx.storage.sql.exec('UPDATE channels SET expires=? WHERE id=?', now + ms, channel),
  };
}

test('frames are forwarded while the channel reservation is current', async (t) => {
  const { host, client, exchange } = setup(t);
  for (let i = 0; i < 3; i++) await exchange();
  assert.equal(client.sent.length, 3, 'data reached the connector');
  assert.equal(host.sent.length, 3, 'credit reached the publisher');
});

test('an expired channel refuses frames even after many forwarded ones', async (t) => {
  const { host, exchange, advance } = setup(t);
  for (let i = 0; i < 5; i++) await exchange();
  advance(30001);
  await exchange();
  assert.equal(host.readyState, WebSocket.CLOSED);
});

test('a renewed channel keeps forwarding past its original expiry', async (t) => {
  const { host, exchange, advance, renew } = setup(t);
  await exchange();
  renew(60000);
  advance(45000);
  await exchange();
  assert.equal(host.readyState, WebSocket.OPEN);
  advance(15001);
  await exchange();
  assert.equal(host.readyState, WebSocket.CLOSED);
});
