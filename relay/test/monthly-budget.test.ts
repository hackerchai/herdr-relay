import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { AdmissionLedger, type LedgerStorage } from '../src/admission-ledger.ts';

function database(): LedgerStorage {
  const db = new DatabaseSync(':memory:');
  return {
    sql: {
      exec<T>(query: string, ...args: (string | number)[]) {
        const rows = db.prepare(query).all(...args) as T[];
        return { toArray: () => rows };
      },
    },
    transactionSync<T>(fn: () => T): T {
      db.exec('BEGIN');
      try {
        const result = fn();
        db.exec('COMMIT');
        return result;
      } catch (error) {
        db.exec('ROLLBACK');
        throw error;
      }
    },
  };
}

const LAST_HOUR_OF_OCTOBER = Date.UTC(2026, 9, 31, 23, 0);
const NOVEMBER = Date.UTC(2026, 10, 1);

function request(ledger: AdmissionLedger, calls = 2) {
  const decision = ledger.begin('ip');
  if (decision.id) ledger.finish(decision.id, 200, calls);
  return decision;
}

test('the monthly Worker request allowance stops new work until the next UTC month', () => {
  const db = database(),
    env = { MONTHLY_WORKER_REQUESTS: '3' };
  let now = LAST_HOUR_OF_OCTOBER,
    ledger = new AdmissionLedger(db, env, () => now);
  assert.equal(ledger.hold('publisher', 'host', 'boot', 600000).code, undefined);
  for (let i = 0; i < 3; i++) assert.equal(request(ledger).code, undefined);
  const stopped = ledger.begin('ip');
  assert.equal(stopped.code, 'GLOBAL_QUOTA_EXCEEDED');
  assert.equal(stopped.budget, true);
  assert.equal(stopped.retry_after, 3600);
  // Existing hosts and channels cannot renew or open either, even after a restart.
  ledger = new AdmissionLedger(db, env, () => now);
  assert.equal(ledger.renew('host', 'boot').code, 'GLOBAL_QUOTA_EXCEEDED');
  assert.equal(ledger.hold('channel', 'c', 'owner', 120000).code, 'GLOBAL_QUOTA_EXCEEDED');
  now = LAST_HOUR_OF_OCTOBER + 3000000;
  assert.equal(ledger.begin('ip').retry_after, 600);
  now = NOVEMBER;
  assert.equal(request(ledger).code, undefined);
});

test('usage is metered as Workers Paid bills it', () => {
  let now = LAST_HOUR_OF_OCTOBER;
  const ledger = new AdmissionLedger(database(), {}, () => now);
  // One entry request: begin and finish are two RPC sessions, plus the two the
  // Worker reports (for example AccessGate and DeviceRelay).
  request(ledger, 2);
  assert.deepEqual(ledger.status().month.usage, {
    worker_requests: 1,
    do_requests: 4,
    do_gb_seconds: 0.005,
    do_rows_written: 16,
  });
  assert.equal(ledger.hold('channel', 'c', 'owner', 120000).code, undefined);
  // 400 incoming WebSocket messages bill as 20 requests and 1 ms each.
  assert.equal(ledger.renew('c', 'owner', { frames: 400, calls: 1 }).code, undefined);
  ledger.release('c', 'owner', { frames: 20 });
  assert.deepEqual(ledger.status().month.usage, {
    worker_requests: 1,
    do_requests: 4 + 1 + (1 + 1 + 20) + (1 + 1),
    do_gb_seconds: (40 + 10 + (20 + 400) + (10 + 20)) / 8000,
    do_rows_written: 16 + 4 + 8 + 4,
  });
  assert.deepEqual(ledger.status().month.limits, {
    worker_requests: 9000000,
    do_requests: 900000,
    do_gb_seconds: 360000,
    do_rows_written: 45000000,
  });
  now = NOVEMBER;
  assert.equal(ledger.status().month.usage.worker_requests, 0);
});

test('Durable Object requests, duration and rows written each stop service at their allowance', () => {
  for (const [key, value, frames] of [
    ['MONTHLY_DO_REQUESTS', '100', 2000],
    ['MONTHLY_DO_GB_SECONDS', '1', 8000],
    ['MONTHLY_DO_ROWS_WRITTEN', '50', 0],
  ] as const) {
    const ledger = new AdmissionLedger(database(), { [key]: value }, () => LAST_HOUR_OF_OCTOBER);
    assert.equal(ledger.hold('channel', 'c', 'owner', 120000).code, undefined, key);
    if (frames) {
      assert.equal(ledger.renew('c', 'owner', { frames: frames / 2 }).code, undefined, key);
      ledger.release('c', 'owner', { frames: frames / 2 });
    } else for (let i = 0; i < 5; i++) request(ledger);
    assert.equal(ledger.begin('ip').code, 'GLOBAL_QUOTA_EXCEEDED', key);
  }
});

test('the monthly budget can be raised or disabled, and invalid settings fail closed', () => {
  const off = new AdmissionLedger(
    database(),
    { MONTHLY_BUDGET: 'off', MONTHLY_WORKER_REQUESTS: '1' },
    () => LAST_HOUR_OF_OCTOBER,
  );
  for (let i = 0; i < 3; i++) assert.equal(request(off).code, undefined);
  const raised = new AdmissionLedger(
    database(),
    { MONTHLY_WORKER_REQUESTS: '20000000' },
    () => LAST_HOUR_OF_OCTOBER,
  );
  assert.equal(raised.status().month.limits.worker_requests, 20000000);
  for (const env of [
    { MONTHLY_BUDGET: 'maybe' },
    { MONTHLY_DO_REQUESTS: '0' },
    { MONTHLY_DO_GB_SECONDS: '1.5' },
  ])
    assert.throws(() => new AdmissionLedger(database(), env), /RELAY_UNCONFIGURED/);
});

test('releases and usage reports are always accepted while service is stopped', () => {
  const ledger = new AdmissionLedger(
    database(),
    { MONTHLY_DO_REQUESTS: '5' },
    () => LAST_HOUR_OF_OCTOBER,
  );
  assert.equal(ledger.hold('channel', 'c', 'owner', 120000).code, undefined);
  ledger.release('c', 'owner', { frames: 200 });
  assert.equal(ledger.begin('ip').code, 'GLOBAL_QUOTA_EXCEEDED');
  assert.deepEqual(ledger.status().active, []);
});
