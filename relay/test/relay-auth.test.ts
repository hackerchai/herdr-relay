import test from 'node:test';
import assert from 'node:assert/strict';
import { platform } from './platform.ts';
const { default: worker } = await import('../src/index.ts');

const TOKEN = 'correct-horse-battery-staple-42';

/** Bindings that fail the test when touched, unless `admit` allows entry. */
function env(origin: string, token: string | undefined, admit = false) {
  let begun = 0;
  const untouchable = {
    getByName: () => assert.fail('a rejected request must not reach a Durable Object'),
  };
  return {
    get begun() {
      return begun;
    },
    env: {
      RELAY_ORIGIN: origin,
      ADMISSION_ENABLED: 'true',
      ...(token === undefined ? {} : { RELAY_ACCESS_TOKEN: token }),
      ADMISSION: admit
        ? {
            getByName: () => ({
              begin: async () => {
                begun++;
                return { code: 'RATE_LIMITED', retry_after: 60 };
              },
              finish: async () => {},
            }),
          }
        : untouchable,
      AUTH: untouchable,
      DEVICES: untouchable,
      ROOMS: untouchable,
    },
  };
}

async function call(origin: string, e: object, authorization?: string) {
  const ctx = platform();
  const response = await worker.fetch(
    new Request(origin + '/v1/challenge', {
      method: 'POST',
      body: '{}',
      headers: authorization ? { Authorization: authorization } : {},
    }),
    e as never,
    ctx as never,
  );
  await ctx.drain();
  return response;
}

test('a self-hosted relay with an access token rejects missing or wrong tokens before any Durable Object', async () => {
  const origin = 'https://relay.example';
  for (const [authorization, code] of [
    [undefined, 'RELAY_AUTH_REQUIRED'],
    ['Bearer wrong-token-of-sufficient-length', 'RELAY_AUTH_INVALID'],
    ['Basic ' + TOKEN, 'RELAY_AUTH_REQUIRED'],
    ['Bearer ' + TOKEN + 'x', 'RELAY_AUTH_INVALID'],
  ] as const) {
    const response = await call(origin, env(origin, TOKEN).env, authorization);
    assert.equal(response.status, 401, String(authorization));
    assert.equal((await response.json()).code, code);
    assert.match(response.headers.get('WWW-Authenticate') ?? '', /^Bearer/);
  }
});

test('the correct token proceeds to admission, and no token configured means open access', async () => {
  const origin = 'https://relay.example';
  const withToken = env(origin, TOKEN, true);
  assert.equal((await call(origin, withToken.env, 'Bearer ' + TOKEN)).status, 429);
  assert.equal(withToken.begun, 1);
  const open = env(origin, undefined, true);
  assert.equal((await call(origin, open.env)).status, 429);
  assert.equal(open.begun, 1);
});

test('the public hosted relay never requires a token', async () => {
  const origin = 'https://herdr-relay.hackerchai.com';
  const configured = env(origin, TOKEN, true);
  assert.equal((await call(origin, configured.env)).status, 429);
  assert.equal(configured.begun, 1);
});

test('a malformed access token configuration fails closed', async () => {
  const origin = 'https://relay.example';
  for (const token of ['short', 'has space inside the token', '']) {
    const response = await call(origin, env(origin, token).env, 'Bearer ' + token);
    assert.equal(response.status, 503, JSON.stringify(token));
    assert.equal((await response.json()).code, 'RELAY_UNCONFIGURED');
  }
});

test('the production deployment is the open public relay with the monthly budget on', async () => {
  const { readFileSync } = await import('node:fs');
  const { PUBLIC_ORIGIN } = await import('../src/protocol.ts');
  const text = readFileSync(new URL('../wrangler.production.jsonc', import.meta.url), 'utf8');
  const config = JSON.parse(text.replace(/,(\s*[}\]])/g, '$1'));
  assert.equal(config.vars.RELAY_ORIGIN, PUBLIC_ORIGIN);
  assert.equal(config.vars.RELAY_ACCESS_TOKEN, undefined);
  assert.equal(config.vars.MONTHLY_BUDGET, 'on');
});

test('the Worker entry exports only handlers and Durable Object classes, as workerd requires', async () => {
  const entry = await import('../src/index.ts');
  for (const [name, value] of Object.entries(entry))
    assert.ok(
      typeof value === 'function' || (name === 'default' && typeof value === 'object'),
      name,
    );
});
