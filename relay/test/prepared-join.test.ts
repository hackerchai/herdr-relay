import test from 'node:test';
import assert from 'node:assert/strict';
import { platform, TestSocket } from './platform.ts';
import { digest } from '../src/protocol.ts';
const { DeviceRelay } = await import('../src/device-relay.ts');

const publisher = 'a'.repeat(64),
  connector = 'c'.repeat(64),
  boot = 'b'.repeat(32),
  channel = 'd'.repeat(32),
  target = 'f'.repeat(32),
  publicKey = '1'.repeat(64);

async function openAttach(features: string[] | undefined) {
  const socket = new TestSocket({
    kind: 'control',
    device: publisher,
    boot,
    ready: true,
    expires: Date.now() + 600000,
    ...(features ? { features, public_key: publicKey } : {}),
  });
  const ctx = platform([socket]);
  const issued: Record<string, unknown>[] = [];
  const env = {
    RELAY_ORIGIN: 'https://relay.example',
    ADMISSION_ENABLED: 'true',
    ROOMS: {
      getByName: () => ({
        async reserve() {
          return true;
        },
        async release() {},
      }),
    },
    ADMISSION: {
      getByName: () => ({
        async hold() {
          return { expires: Date.now() + 30000 };
        },
        async release() {},
      }),
    },
    AUTH: {
      getByName: (name: string) => ({
        async issue(value: Record<string, unknown>) {
          issued.push({ name, ...value });
          return { ...value, nonce: '7'.repeat(32), expires_at: '9999999999' };
        },
      }),
    },
  };
  const relay = new DeviceRelay(ctx, env);
  ctx.storage.sql.exec('INSERT INTO grants VALUES (?,1)', connector);
  ctx.storage.sql.exec('INSERT INTO targets VALUES (?)', target);
  const response = await relay.fetch(
    new Request(`https://relay.example/v1/devices/${publisher}/access`, {
      method: 'POST',
      headers: { 'X-Relay-Device': connector },
      body: JSON.stringify({ channel_id: channel, purpose: 'attach', target_id: target }),
    }),
  );
  await ctx.drain();
  assert.equal(response.status, 200);
  return { event: JSON.parse(socket.sent.at(-1)!), issued };
}

test('the publisher receives a pre-issued challenge for joining the channel it is offered', async () => {
  const { event, issued } = await openAttach(['prepared-join']);
  assert.equal(event.type, 'access');
  const path = `/v1/devices/${publisher}/channels/${channel}/publisher`;
  assert.equal(event.join_challenge.path, path);
  assert.equal(event.join_challenge.method, 'GET');
  assert.equal(event.join_challenge.payload_sha256, await digest(event.ticket));
  assert.equal(event.join_challenge.public_key, publicKey);
  assert.deepEqual(
    issued.map((i) => i.name),
    ['device:' + publisher],
  );
});

test('a publisher without the capability receives the legacy access event', async () => {
  const { event, issued } = await openAttach(undefined);
  assert.equal(event.type, 'access');
  assert.equal('join_challenge' in event, false);
  assert.deepEqual(issued, []);
});

async function workerAccess(features: string | undefined) {
  const { default: worker } = await import('../src/index.ts');
  const ticket = '8'.repeat(64);
  const body = JSON.stringify({ channel_id: channel, purpose: 'attach', target_id: target });
  const path = `/v1/devices/${publisher}/access`;
  const proof = {
    challenge: {
      version: 1,
      origin: 'https://relay.example',
      method: 'POST',
      path,
      device_id: connector,
      public_key: publicKey,
      nonce: '2'.repeat(32),
      expires_at: '9999999999',
      payload_sha256: await digest(body),
    },
    signature: '3'.repeat(128),
  };
  const forwarded: Headers[] = [];
  const issued: Record<string, unknown>[] = [];
  const stub = () => ({});
  const env = {
    RELAY_ORIGIN: 'https://relay.example',
    ADMISSION_ENABLED: 'true',
    ROOMS: { getByName: stub },
    ADMISSION: {
      getByName: () => ({
        async begin() {
          return { id: 'lease' };
        },
        async finish() {},
      }),
    },
    AUTH: {
      getByName: (name: string) => ({
        async consume() {
          return connector;
        },
        async issue(value: Record<string, unknown>) {
          issued.push({ name, ...value });
          return { ...value, nonce: '7'.repeat(32), expires_at: '9999999999' };
        },
      }),
    },
    DEVICES: {
      getByName: () => ({
        async fetch(request: Request) {
          forwarded.push(request.headers);
          return Response.json({
            channel_id: channel,
            boot_id: boot,
            ticket,
            grant_version: 1,
            limits: { bytes_per_second: 262144 },
          });
        },
      }),
    },
  };
  const headers: Record<string, string> = {
    'X-Relay-Proof': JSON.stringify(proof),
    // A client cannot choose the key the relay later signs challenges for.
    'X-Relay-Public-Key': '9'.repeat(64),
  };
  if (features) headers['X-Relay-Features'] = features;
  const response = await worker.fetch(
    new Request('https://relay.example' + path, { method: 'POST', headers, body }),
    env as never,
    { waitUntil() {} } as never,
  );
  assert.equal(response.status, 200);
  type Opened = { join_challenge: Record<string, string> };
  return { opened: (await response.json()) as Opened, forwarded, issued, ticket };
}

test('a connector receives a pre-issued challenge for joining the channel it opened', async () => {
  const { opened, forwarded, issued, ticket } = await workerAccess('stream-v2,prepared-join');
  assert.equal(forwarded[0].get('X-Relay-Public-Key'), publicKey);
  assert.equal(
    opened.join_challenge.path,
    `/v1/devices/${publisher}/channels/${channel}/connector`,
  );
  assert.equal(opened.join_challenge.device_id, connector);
  assert.equal(opened.join_challenge.payload_sha256, await digest(ticket));
  assert.deepEqual(
    issued.map((i) => i.name),
    ['device:' + connector],
  );
});

test('a connector without the capability receives the legacy access response', async () => {
  const { opened, issued } = await workerAccess(undefined);
  assert.equal('join_challenge' in opened, false);
  assert.deepEqual(issued, []);
});
