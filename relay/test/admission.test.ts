import test from 'node:test';
import assert from 'node:assert/strict';
import { DatabaseSync } from 'node:sqlite';
import { AdmissionLedger, type LedgerStorage } from '../src/admission-ledger.ts';

// Execute the ledger against SQLite, including real rollback and reopening the
// ledger over the same persisted database. No imitation of its quota logic.
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

test('global entry quota survives reconstruction and bounds unauthenticated work across IPs', () => {
  const db = database(),
    env = {
      GLOBAL_REQUESTS_PER_MINUTE: '3',
      GLOBAL_IP_REQUESTS_PER_MINUTE: '2',
      GLOBAL_PENDING_REQUESTS: '1',
    };
  let now = 100000,
    ledger = new AdmissionLedger(db, env, () => now);
  const first = ledger.begin('ip-a');
  assert.equal(first.code, undefined);
  assert.equal(ledger.begin('ip-b').code, 'RELAY_BUSY');
  ledger.finish(first.id!, 200);
  const second = ledger.begin('ip-a');
  ledger.finish(second.id!, 401);
  assert.equal(ledger.begin('ip-a').code, 'RATE_LIMITED');
  ledger = new AdmissionLedger(db, env, () => now);
  const third = ledger.begin('new-ip');
  assert.equal(third.code, undefined);
  ledger.finish(third.id!, 400);
  assert.equal(ledger.begin('another-new-ip').code, 'RATE_LIMITED');
  now += 60001;
  assert.equal(ledger.begin('fresh-ip').code, undefined);
  now += 15001; // A crashed/slow request cannot hold the slot forever.
  assert.equal(ledger.begin('next-ip').code, undefined);
});

test('leases retain global capacity and prepaid daily time across restarts, midnight and late releases', () => {
  const db = database(),
    env = {
      GLOBAL_CHANNELS: '1',
      GLOBAL_PENDING_CHANNELS: '1',
      GLOBAL_CHANNEL_SECONDS_PER_DAY: '150',
    };
  let now = 86400000 - 60000,
    ledger = new AdmissionLedger(db, env, () => now);
  assert.equal(ledger.hold('channel', 'a', 'owner-a', 120000).code, undefined);
  assert.equal(ledger.hold('channel', 'b', 'owner-b', 120000).code, 'GLOBAL_QUOTA_EXCEEDED');
  ledger = new AdmissionLedger(db, env, () => now);
  assert.equal(ledger.hold('channel', 'b', 'owner-b', 120000).code, 'GLOBAL_QUOTA_EXCEEDED');
  ledger.release('a', 'wrong-owner');
  assert.equal(ledger.hold('channel', 'b', 'owner-b', 120000).code, 'GLOBAL_QUOTA_EXCEEDED');
  now += 30000;
  assert.equal(ledger.renew('a', 'owner-a').expires, now + 120000);
  ledger.release('a', 'owner-a');
  // Releasing early does not refund the reservation or permit repeated short
  // channels to evade the daily quota. The first lease spent 90 s tomorrow.
  assert.equal(ledger.hold('channel', 'b', 'owner-b', 120000).code, 'GLOBAL_QUOTA_EXCEEDED');
  // A rejected reservation must not partially spend today's remaining time.
  assert.equal(ledger.hold('channel', 'b', 'owner-b', 30000).code, undefined);
  now += 30001;
  assert.equal(ledger.renew('b', 'owner-b').code, 'CHANNEL_EXPIRED');
  assert.equal(ledger.hold('publisher', 'publisher', 'boot-1', 120000).code, undefined);
  assert.equal(ledger.hold('publisher', 'publisher', 'boot-2', 120000).code, undefined);
  ledger.release('publisher', 'boot-1');
  assert.equal(ledger.renew('publisher', 'boot-2').code, undefined);
});
