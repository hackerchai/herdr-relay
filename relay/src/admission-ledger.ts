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
    | 'GLOBAL_CHANNEL_SECONDS_PER_DAY',
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
export interface LedgerStorage {
  sql: {
    exec<T extends Record<string, string | number | null> = Record<string, string | number | null>>(
      query: string,
      ...args: (string | number)[]
    ): { toArray(): T[] };
  };
  transactionSync<T>(fn: () => T): T;
}
export type Decision = { code?: string; retry_after?: number; id?: string; expires?: number };
type Lease = { id: string; owner: string; kind: string; expires: number; pending: number };
export type LeaseKind = 'challenge' | 'publisher' | 'channel';
export const PUBLISHER_LEASE_MS = 600000;
const DAY = 86400000;
export class AdmissionLedger {
  private db: LedgerStorage;
  private limits: ReturnType<typeof globalQuotas>;
  private clock: () => number;
  constructor(db: LedgerStorage, env: AdmissionEnv, clock: () => number = Date.now) {
    this.db = db;
    this.limits = globalQuotas(env);
    this.clock = clock;
    db.sql.exec(
      'CREATE TABLE IF NOT EXISTS admission_buckets (key TEXT NOT NULL, period INTEGER NOT NULL, value INTEGER NOT NULL, PRIMARY KEY(key,period))',
    );
    db.sql.exec(
      'CREATE TABLE IF NOT EXISTS admission_leases (id TEXT PRIMARY KEY, owner TEXT NOT NULL, kind TEXT NOT NULL, expires INTEGER NOT NULL, pending INTEGER NOT NULL)',
    );
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
  finish(id: string, status: number): void {
    this.db.sql.exec("DELETE FROM admission_leases WHERE id=? AND kind='request'", id);
    this.add(
      status >= 500 ? 'server_errors' : status >= 400 ? 'client_errors' : 'completed',
      Math.floor(this.clock() / DAY),
    );
  }
  hold(kind: LeaseKind, id: string, owner: string, duration: number): Decision {
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
  renew(id: string, owner: string): Decision {
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
    return this.hold(
      row.kind as LeaseKind,
      id,
      owner,
      row.kind === 'publisher' ? PUBLISHER_LEASE_MS : 120000,
    );
  }
  authenticated(id: string, owner: string): void {
    this.db.sql.exec('UPDATE admission_leases SET pending=0 WHERE id=? AND owner=?', id, owner);
  }
  release(id: string, owner: string): void {
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
    };
  }
}
