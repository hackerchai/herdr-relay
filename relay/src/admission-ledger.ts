import { ProtocolError, randomHex } from './protocol.ts';

export type AdmissionEnv = Partial<
  Record<
    | 'GLOBAL_REQUESTS_PER_MINUTE'
    | 'GLOBAL_REQUESTS_PER_DAY'
    | 'GLOBAL_IP_REQUESTS_PER_MINUTE'
    | 'GLOBAL_PENDING_REQUESTS'
    | 'GLOBAL_CHALLENGES'
    | 'GLOBAL_PUBLISHERS'
    | 'GLOBAL_CHANNELS'
    | 'GLOBAL_PENDING_CHANNELS'
    | 'GLOBAL_CHANNEL_SECONDS_PER_DAY'
    | 'MONTHLY_BUDGET'
    | 'MONTHLY_WORKER_REQUESTS'
    | 'MONTHLY_DO_REQUESTS'
    | 'MONTHLY_DO_GB_SECONDS'
    | 'MONTHLY_DO_ROWS_WRITTEN',
    string
  >
>;
export function globalQuotas(env: AdmissionEnv) {
  const value = (key: keyof AdmissionEnv, fallback: number): number => {
    const text = env[key];
    if (text === undefined) return fallback;
    const n = Number(text);
    if (!/^[1-9][0-9]*$/.test(text) || !Number.isSafeInteger(n) || n > fallback)
      throw new ProtocolError('RELAY_UNCONFIGURED', 503);
    return n;
  };
  return {
    minute: value('GLOBAL_REQUESTS_PER_MINUTE', 600),
    day: value('GLOBAL_REQUESTS_PER_DAY', 100000),
    ip: value('GLOBAL_IP_REQUESTS_PER_MINUTE', 240),
    request: value('GLOBAL_PENDING_REQUESTS', 32),
    challenge: value('GLOBAL_CHALLENGES', 256),
    publisher: value('GLOBAL_PUBLISHERS', 64),
    channel: value('GLOBAL_CHANNELS', 32),
    pending: value('GLOBAL_PENDING_CHANNELS', 64),
    channel_ms: value('GLOBAL_CHANNEL_SECONDS_PER_DAY', 345600) * 1000,
  };
}
/**
 * Monthly allowances, by default 90% of what Workers Paid includes, leaving
 * headroom for work the meter cannot see (CPU time, pings, isolate-cached
 * rejections). Unlike the engineering quotas above they may be raised.
 */
export function monthlyBudget(env: AdmissionEnv) {
  const value = (key: keyof AdmissionEnv, fallback: number): number => {
    const text = env[key];
    if (text === undefined) return fallback;
    const n = Number(text);
    if (!/^[1-9][0-9]*$/.test(text) || !Number.isSafeInteger(n))
      throw new ProtocolError('RELAY_UNCONFIGURED', 503);
    return n;
  };
  if (![undefined, 'on', 'off'].includes(env.MONTHLY_BUDGET))
    throw new ProtocolError('RELAY_UNCONFIGURED', 503);
  return {
    enabled: env.MONTHLY_BUDGET !== 'off',
    limits: {
      worker_requests: value('MONTHLY_WORKER_REQUESTS', 9000000),
      do_requests: value('MONTHLY_DO_REQUESTS', 900000),
      do_gb_seconds: value('MONTHLY_DO_GB_SECONDS', 360000),
      do_rows_written: value('MONTHLY_DO_ROWS_WRITTEN', 45000000),
    },
  };
}
/**
 * Work a Durable Object reports alongside an Admission call: incoming
 * WebSocket messages and other Durable Object invocations it caused.
 */
export interface Usage {
  frames?: number;
  calls?: number;
}
// The billing model. Every Durable Object invocation (RPC session, fetch or
// alarm) is one request, charged a conservative 10 ms of 128 MB wall time and
// 4 SQLite rows written. Incoming WebSocket messages bill at 20:1 and 1 ms each.
const CALL_MS = 10;
const CALL_ROWS = 4;
const FRAME_MS = 1;
const FRAMES_PER_REQUEST = 20;
const MS_PER_GB_SECOND = 8000;
const MAX_REPORT = 100000000;
function month(now: number): number {
  const date = new Date(now);
  return date.getUTCFullYear() * 12 + date.getUTCMonth();
}
function monthEnd(period: number): number {
  return Date.UTC(Math.floor(period / 12), (period % 12) + 1, 1);
}
export interface LedgerStorage {
  sql: {
    exec<T extends Record<string, string | number | null> = Record<string, string | number | null>>(
      query: string,
      ...args: (string | number)[]
    ): { toArray(): T[] };
  };
  transactionSync<T>(fn: () => T): T;
}
export type Decision = {
  code?: string;
  retry_after?: number;
  id?: string;
  expires?: number;
  /** Set when the monthly budget, not a short-term quota, refused the work. */
  budget?: boolean;
};
type Lease = { id: string; owner: string; kind: string; expires: number; pending: number };
export type LeaseKind = 'challenge' | 'publisher' | 'channel';
export const PUBLISHER_LEASE_MS = 600000;
const DAY = 86400000;
export class AdmissionLedger {
  private db: LedgerStorage;
  private limits: ReturnType<typeof globalQuotas>;
  private budget: ReturnType<typeof monthlyBudget>;
  private clock: () => number;
  constructor(db: LedgerStorage, env: AdmissionEnv, clock: () => number = Date.now) {
    this.db = db;
    this.limits = globalQuotas(env);
    this.budget = monthlyBudget(env);
    this.clock = clock;
    db.sql.exec(
      'CREATE TABLE IF NOT EXISTS admission_buckets (key TEXT NOT NULL, period INTEGER NOT NULL, value INTEGER NOT NULL, PRIMARY KEY(key,period))',
    );
    db.sql.exec(
      'CREATE TABLE IF NOT EXISTS admission_leases (id TEXT PRIMARY KEY, owner TEXT NOT NULL, kind TEXT NOT NULL, expires INTEGER NOT NULL, pending INTEGER NOT NULL)',
    );
    // One row per UTC month; requests are stored in twentieths so that
    // WebSocket messages stay integral.
    db.sql.exec(
      'CREATE TABLE IF NOT EXISTS admission_usage (period INTEGER PRIMARY KEY, worker_requests INTEGER NOT NULL, do_request_twentieths INTEGER NOT NULL, do_ms INTEGER NOT NULL, do_rows_written INTEGER NOT NULL)',
    );
  }
  /** Record one invocation of this object plus the reported usage. */
  charge(usage: Usage = {}, workerRequests = 0): void {
    // Never fail the call carrying a report (a release must always succeed).
    const count = (n: number | undefined) =>
      n !== undefined && Number.isSafeInteger(n) && n > 0 ? Math.min(n, MAX_REPORT) : 0;
    const frames = count(usage.frames),
      calls = 1 + count(usage.calls);
    this.db.sql.exec(
      'INSERT INTO admission_usage VALUES (?,?,?,?,?) ON CONFLICT(period) DO UPDATE SET worker_requests=worker_requests+excluded.worker_requests, do_request_twentieths=do_request_twentieths+excluded.do_request_twentieths, do_ms=do_ms+excluded.do_ms, do_rows_written=do_rows_written+excluded.do_rows_written',
      month(this.clock()),
      workerRequests,
      calls * FRAMES_PER_REQUEST + frames,
      calls * CALL_MS + frames * FRAME_MS,
      calls * CALL_ROWS,
    );
  }
  private usage() {
    const row = this.db.sql
      .exec<{
        worker_requests: number;
        do_request_twentieths: number;
        do_ms: number;
        do_rows_written: number;
      }>('SELECT * FROM admission_usage WHERE period=?', month(this.clock()))
      .toArray()[0];
    return {
      worker_requests: row?.worker_requests ?? 0,
      do_requests: (row?.do_request_twentieths ?? 0) / FRAMES_PER_REQUEST,
      do_gb_seconds: (row?.do_ms ?? 0) / MS_PER_GB_SECOND,
      do_rows_written: row?.do_rows_written ?? 0,
    };
  }
  /** A refusal once any monthly allowance is spent, retried at most hourly. */
  private exhausted(): Decision | undefined {
    if (!this.budget.enabled) return;
    const usage = this.usage();
    const limits = this.budget.limits;
    if ((Object.keys(limits) as (keyof typeof limits)[]).every((key) => usage[key] < limits[key]))
      return;
    const now = this.clock();
    this.add('rejected_MONTHLY_BUDGET', Math.floor(now / DAY));
    return {
      code: 'GLOBAL_QUOTA_EXCEEDED',
      retry_after: Math.min(3600, Math.ceil((monthEnd(month(now)) - now) / 1000)),
      budget: true,
    };
  }
  private count(key: string, period: number): number {
    return (
      this.db.sql
        .exec<{ value: number }>(
          'SELECT value FROM admission_buckets WHERE key=? AND period=?',
          key,
          period,
        )
        .toArray()[0]?.value ?? 0
    );
  }
  private add(key: string, period: number, n = 1): void {
    this.db.sql.exec(
      'INSERT INTO admission_buckets VALUES (?,?,?) ON CONFLICT(key,period) DO UPDATE SET value=MIN(value+excluded.value,9007199254740991)',
      key,
      period,
      n,
    );
  }
  private active(kind: string): number {
    return this.db.sql
      .exec<{ n: number }>('SELECT COUNT(*) AS n FROM admission_leases WHERE kind=?', kind)
      .toArray()[0].n;
  }
  cleanup(): void {
    const now = this.clock();
    this.db.sql.exec('DELETE FROM admission_leases WHERE expires<=?', now);
    this.db.sql.exec(
      "DELETE FROM admission_buckets WHERE (key LIKE 'minute%' AND period<?) OR (key NOT LIKE 'minute%' AND period<?)",
      Math.floor(now / 60000) - 1,
      Math.floor(now / DAY) - 2,
    );
    this.db.sql.exec('DELETE FROM admission_usage WHERE period<?', month(now) - 1);
  }
  nextDeadline(): number | null {
    return this.db.sql
      .exec<{ deadline: number | null }>(
        "SELECT MIN(deadline) AS deadline FROM (SELECT expires AS deadline FROM admission_leases UNION ALL SELECT CASE WHEN key LIKE 'minute%' THEN (period+2)*60000 ELSE (period+3)*86400000 END AS deadline FROM admission_buckets)",
      )
      .toArray()[0].deadline;
  }
  begin(ip: string): Decision {
    return this.db.transactionSync(() => {
      this.cleanup();
      const spent = this.exhausted();
      this.charge({}, 1);
      if (spent) return spent;
      const now = this.clock(),
        minute = Math.floor(now / 60000),
        day = Math.floor(now / DAY);
      let code: string | undefined,
        retry = 60;
      if (this.count('requests', day) >= this.limits.day) {
        code = 'GLOBAL_QUOTA_EXCEEDED';
        retry = Math.ceil(((day + 1) * DAY - now) / 1000);
      } else if (
        this.count('minute', minute) >= this.limits.minute ||
        this.count('minute:' + ip, minute) >= this.limits.ip
      )
        code = 'RATE_LIMITED';
      else if (this.active('request') >= this.limits.request) {
        code = 'RELAY_BUSY';
        retry = 15;
      }
      if (code) {
        this.add('rejected_' + code, day);
        return { code, retry_after: retry };
      }
      this.add('requests', day);
      this.add('minute', minute);
      this.add('minute:' + ip, minute);
      const id = randomHex(),
        expires = now + 15000;
      this.db.sql.exec("INSERT INTO admission_leases VALUES (?,'','request',?,0)", id, expires);
      return { id, expires };
    });
  }
  finish(id: string, status: number, calls = 0): void {
    this.charge({ calls });
    this.db.sql.exec("DELETE FROM admission_leases WHERE id=? AND kind='request'", id);
    this.add(
      status >= 500 ? 'server_errors' : status >= 400 ? 'client_errors' : 'completed',
      Math.floor(this.clock() / DAY),
    );
  }
  hold(kind: LeaseKind, id: string, owner: string, duration: number, usage?: Usage): Decision {
    this.charge(usage);
    return this.reserve(kind, id, owner, duration);
  }
  private reserve(kind: LeaseKind, id: string, owner: string, duration: number): Decision {
    if (
      !['challenge', 'publisher', 'channel'].includes(kind) ||
      !Number.isInteger(duration) ||
      duration < 1 ||
      duration > (kind === 'publisher' ? PUBLISHER_LEASE_MS : 120000) ||
      id.length > 160 ||
      owner.length > 64
    )
      throw new ProtocolError();
    return this.db.transactionSync(() => {
      this.cleanup();
      const spent = this.exhausted();
      if (spent) return spent;
      const now = this.clock(),
        old = this.db.sql.exec<Lease>('SELECT * FROM admission_leases WHERE id=?', id).toArray()[0];
      if (old && (old.kind !== kind || (old.owner !== owner && kind !== 'publisher')))
        return { code: 'CHANNEL_CONFLICT', retry_after: 120 };
      const reject = (retry = 120) => {
        this.add('rejected_GLOBAL_QUOTA_EXCEEDED', Math.floor(now / DAY));
        return { code: 'GLOBAL_QUOTA_EXCEEDED', retry_after: retry };
      };
      if (!old && this.active(kind) >= this.limits[kind]) return reject();
      if (
        kind === 'channel' &&
        !old &&
        this.db.sql
          .exec<{ n: number }>('SELECT COUNT(*) AS n FROM admission_leases WHERE pending=1')
          .toArray()[0].n >= this.limits.pending
      )
        return reject();
      const expires = Math.max(old?.expires ?? 0, now + duration);
      const charges: { day: number; ms: number }[] = [];
      if (kind === 'publisher' || kind === 'channel') {
        // Account for newly allocated time across UTC midnight. Publisher
        // presence is bounded by concurrent leases, never a daily uptime cap.
        // Data channels still prepay time without refunds for early release.
        const key = (kind + '_ms') as 'publisher_ms' | 'channel_ms';
        for (let start = Math.max(now, old?.expires ?? 0); start < expires;) {
          const day = Math.floor(start / DAY),
            end = Math.min(expires, (day + 1) * DAY),
            ms = end - start;
          if (kind === 'channel' && this.count(key, day) + ms > this.limits.channel_ms)
            return reject(Math.ceil(((day + 1) * DAY - now) / 1000));
          charges.push({ day, ms });
          start = end;
        }
        for (const c of charges) this.add(key, c.day, c.ms);
      }
      if (!old) this.add('opened_' + kind, Math.floor(now / DAY));
      this.db.sql.exec(
        'INSERT OR REPLACE INTO admission_leases VALUES (?,?,?,?,?)',
        id,
        owner,
        kind,
        expires,
        old?.pending ?? (kind === 'channel' ? 1 : 0),
      );
      return { id, expires };
    });
  }
  renew(id: string, owner: string, usage?: Usage): Decision {
    this.charge(usage);
    const row = this.db.sql
      .exec<Lease>(
        'SELECT * FROM admission_leases WHERE id=? AND owner=? AND expires>?',
        id,
        owner,
        this.clock(),
      )
      .toArray()[0];
    if (!row || !['publisher', 'channel'].includes(row.kind))
      return { code: 'CHANNEL_EXPIRED', retry_after: 120 };
    return this.reserve(
      row.kind as LeaseKind,
      id,
      owner,
      row.kind === 'publisher' ? PUBLISHER_LEASE_MS : 120000,
    );
  }
  authenticated(id: string, owner: string): void {
    this.db.sql.exec('UPDATE admission_leases SET pending=0 WHERE id=? AND owner=?', id, owner);
  }
  release(id: string, owner: string, usage?: Usage): void {
    this.charge(usage);
    this.db.sql.exec('DELETE FROM admission_leases WHERE id=? AND owner=?', id, owner);
  }
  status() {
    this.cleanup();
    return {
      at: this.clock(),
      quotas: this.limits,
      active: this.db.sql
        .exec<{ kind: string; count: number }>(
          'SELECT kind,COUNT(*) AS count FROM admission_leases GROUP BY kind',
        )
        .toArray(),
      pending_channels: this.db.sql
        .exec<{ count: number }>('SELECT COUNT(*) AS count FROM admission_leases WHERE pending=1')
        .toArray()[0].count,
      daily: this.db.sql
        .exec<{ key: string; period: number; value: number }>(
          "SELECT * FROM admission_buckets WHERE key NOT LIKE 'minute%' ORDER BY period,key",
        )
        .toArray(),
      month: { enabled: this.budget.enabled, usage: this.usage(), limits: this.budget.limits },
    };
  }
}
