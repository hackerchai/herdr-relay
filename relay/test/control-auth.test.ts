import test from 'node:test';
import assert from 'node:assert/strict';
import { platform, TestSocket } from './platform.ts';
const { DeviceRelay } = await import('../src/device-relay.ts');

const device = 'a'.repeat(64),
  connector = 'c'.repeat(64),
  boot = 'b'.repeat(32),
  channel = 'd'.repeat(32),
  reservation = 'e'.repeat(32);

function setup(features: string[] | undefined, expires = Date.now() + 600000) {
  const socket = new TestSocket({
    kind: 'control',
    device,
    boot,
    ready: true,
    expires,
    ...(features ? { features } : {}),
  });
  const ctx = platform([socket]);
  const admitted: string[] = [];
  const env = {
    ADMISSION_ENABLED: 'true',
    ROOMS: { getByName: () => ({ async release() {}, async renew() {} }) },
    ADMISSION: {
      getByName: () => ({
        async authenticated(id: string) {
          admitted.push(id);
        },
        async release() {},
        async renew() {
          return { expires: Date.now() + 600000 };
        },
      }),
    },
  };
  const relay = new DeviceRelay(ctx, env);
  ctx.storage.sql.exec(
    "INSERT INTO channels (id,reservation,publisher,connector,boot,host_ticket,client_ticket,host_used,client_used,expires,purpose,target,grant_version,authenticated,rate,window) VALUES (?,?,?,?,?,'h','c',1,1,?,'attach',?,1,0,262144,0)",
    channel,
    reservation,
    device,
    connector,
    boot,
    Date.now() + 30000,
    'f'.repeat(32),
  );
  return { socket, ctx, relay, admitted };
}
const authenticated = (channel_id: string) =>
  JSON.stringify({ type: 'authenticated', channel_id, boot_id: boot });

test('a publisher reports channel authentication on its control socket without a separate request', async () => {
  const { socket, ctx, relay, admitted } = setup(['control-authenticated']);
  await relay.webSocketMessage(socket as unknown as WebSocket, authenticated(channel));
  await ctx.drain();
  assert.deepEqual(admitted, ['channel:' + device + ':' + channel]);
  assert.equal(
    ctx.storage.sql.exec('SELECT authenticated FROM channels WHERE id=?', channel).one()
      .authenticated,
    1,
  );
  assert.deepEqual(JSON.parse(socket.sent.at(-1)!), { type: 'authenticated', channel_id: channel });
  assert.equal(socket.readyState, WebSocket.OPEN);
});

test('an unknown channel is refused without ending the publisher', async () => {
  const { socket, ctx, relay, admitted } = setup(['control-authenticated']);
  await relay.webSocketMessage(socket as unknown as WebSocket, authenticated('9'.repeat(32)));
  await ctx.drain();
  assert.deepEqual(admitted, []);
  assert.deepEqual(JSON.parse(socket.sent.at(-1)!), {
    type: 'authentication_failed',
    channel_id: '9'.repeat(32),
    code: 'NOT_AUTHORIZED',
  });
  assert.equal(socket.readyState, WebSocket.OPEN);
});

test('a publisher that did not negotiate the capability keeps the legacy control contract', async () => {
  const { socket, ctx, relay, admitted } = setup(undefined);
  await relay.webSocketMessage(socket as unknown as WebSocket, authenticated(channel));
  await ctx.drain();
  assert.deepEqual(admitted, []);
  assert.equal(socket.readyState, WebSocket.CLOSED);
});

test('a publisher whose admission lease expired cannot acknowledge a channel', async () => {
  const { socket, ctx, relay, admitted } = setup(['control-authenticated'], Date.now() + 1000);
  socket.attachment.expires = Date.now() - 1;
  await relay.webSocketMessage(socket as unknown as WebSocket, authenticated(channel));
  await ctx.drain();
  assert.deepEqual(admitted, []);
  assert.ok(!socket.sent.some((m) => JSON.parse(m).type === 'authenticated'));
});
