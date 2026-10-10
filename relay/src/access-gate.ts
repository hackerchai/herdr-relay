import { DurableObject } from 'cloudflare:workers';
import {
  type Challenge,
  type Proof,
  ProtocolError,
  canonicalAccess,
  challenge,
  digest,
  bytes,
  randomHex,
  record,
  verifyAccess,
} from './protocol.ts';
import type { Env } from './index.ts';
import { ADMISSION_OBJECT, admitted } from './admission.ts';

export class AccessGate extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS challenges (nonce TEXT PRIMARY KEY, value TEXT NOT NULL, expires INTEGER NOT NULL)',
    );
    if (
      !ctx.storage.sql
        .exec<{ name: string }>('PRAGMA table_info(challenges)')
        .toArray()
        .some((c) => c.name === 'prepared')
    )
      ctx.storage.sql.exec('ALTER TABLE challenges ADD COLUMN prepared INTEGER NOT NULL DEFAULT 0');
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS rate (id INTEGER PRIMARY KEY CHECK(id=1), start INTEGER NOT NULL, count INTEGER NOT NULL)',
    );
  }
  async budget(limit: number): Promise<void> {
    const now = Date.now();
    this.ctx.storage.transactionSync(() => {
      const row = this.ctx.storage.sql
        .exec<{ start: number; count: number }>('SELECT start,count FROM rate WHERE id=1')
        .toArray()[0];
      if (!row || row.start + 60000 <= now)
        this.ctx.storage.sql.exec('INSERT OR REPLACE INTO rate VALUES (1,?,1)', now);
      else {
        if (row.count >= limit) throw new ProtocolError('RATE_LIMITED', 429);
        this.ctx.storage.sql.exec('UPDATE rate SET count=count+1 WHERE id=1');
      }
    });
    await this.arm(now + 60000);
  }
  /** `prepared` challenges are issued in advance for joins and use their own
   *  small pool, so unredeemed ones never block a device's fresh requests. */
  async issue(input: unknown, prepared = false): Promise<Challenge> {
    const r = record(input, [
      'version',
      'origin',
      'method',
      'path',
      'device_id',
      'public_key',
      'payload_sha256',
    ]);
    const now = Math.floor(Date.now() / 1000);
    const c = challenge({ ...r, nonce: randomHex(), expires_at: String(now + 60) });
    if (
      c.origin !== this.env.RELAY_ORIGIN ||
      (await digest(bytes('302a300506032b6570032100' + c.public_key))) !== c.device_id
    )
      throw new ProtocolError('ACCESS_INVALID', 401);
    const admission = this.env.ADMISSION.getByName(ADMISSION_OBJECT),
      id = 'nonce:' + c.device_id + ':' + c.nonce;
    const diagnostic =
      c.path === '/v1/operator/status' &&
      !!this.env.OPERATOR_DEVICE_IDS?.split(',').includes(c.device_id);
    admitted(await admission.hold('challenge', id, c.nonce, 60000, diagnostic));
    try {
      this.ctx.storage.transactionSync(() => {
        this.ctx.storage.sql.exec('DELETE FROM challenges WHERE expires<=?', now);
        const { count } = this.ctx.storage.sql
          .exec<{
            count: number;
          }>('SELECT COUNT(*) AS count FROM challenges WHERE prepared=?', prepared ? 1 : 0)
          .one();
        if (count >= (prepared ? 8 : 4)) throw new ProtocolError('RATE_LIMITED', 429);
        this.ctx.storage.sql.exec(
          'INSERT INTO challenges (nonce,value,expires,prepared) VALUES (?,?,?,?)',
          c.nonce,
          JSON.stringify(c),
          now + 60,
          prepared ? 1 : 0,
        );
      });
    } catch (error) {
      await admission.release(id, c.nonce);
      throw error;
    }
    await this.arm((now + 60) * 1000);
    return c;
  }
  async consume(proof: Proof): Promise<string> {
    if (!(await verifyAccess(proof, this.env.RELAY_ORIGIN, Math.floor(Date.now() / 1000))))
      throw new ProtocolError('ACCESS_INVALID', 401);
    const device = this.ctx.storage.transactionSync(() => {
      const c = proof.challenge;
      const row = this.ctx.storage.sql
        .exec<{ value: string; expires: number }>(
          'SELECT value,expires FROM challenges WHERE nonce=?',
          c.nonce,
        )
        .toArray()[0];
      if (
        !row ||
        row.expires <= Math.floor(Date.now() / 1000) ||
        canonicalAccess(JSON.parse(row.value)) !== canonicalAccess(c)
      )
        throw new ProtocolError('ACCESS_REPLAYED', 401);
      this.ctx.storage.sql.exec('DELETE FROM challenges WHERE nonce=?', c.nonce);
      return c.device_id;
    });
    await this.env.ADMISSION.getByName(ADMISSION_OBJECT).release(
      'nonce:' + device + ':' + proof.challenge.nonce,
      proof.challenge.nonce,
    );
    return device;
  }
  async alarm(): Promise<void> {
    const now = Date.now();
    this.ctx.storage.sql.exec('DELETE FROM challenges WHERE expires<=?', Math.floor(now / 1000));
    this.ctx.storage.sql.exec('DELETE FROM rate WHERE start+60000<=?', now);
    const expirations = this.ctx.storage.sql
      .exec<{ expires: number }>(
        'SELECT expires*1000 AS expires FROM challenges UNION ALL SELECT start+60000 AS expires FROM rate',
      )
      .toArray();
    if (expirations.length)
      await this.ctx.storage.setAlarm(Math.min(...expirations.map((e) => e.expires)));
  }
  private async arm(deadline: number): Promise<void> {
    const current = await this.ctx.storage.getAlarm();
    if (current === null || current > deadline) await this.ctx.storage.setAlarm(deadline);
  }
}
