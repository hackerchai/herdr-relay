import { DurableObject } from 'cloudflare:workers';
import { ProtocolError, isHex } from './protocol.ts';
import type { Env } from './index.ts';

/** One opaque target's cloud reservation. Only DeviceRelay calls this RPC. */
export class Room extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    ctx.storage.sql.exec(
      'CREATE TABLE IF NOT EXISTS occupancy (slot INTEGER PRIMARY KEY CHECK(slot=1), reservation TEXT NOT NULL, expires INTEGER NOT NULL)',
    );
  }
  async reserve(reservation: string, expires: number): Promise<boolean> {
    if (
      !isHex(reservation, 32) ||
      !Number.isSafeInteger(expires) ||
      expires <= Date.now() ||
      expires > Date.now() + 120000
    )
      throw new ProtocolError();
    const accepted = this.ctx.storage.transactionSync(() => {
      this.ctx.storage.sql.exec('DELETE FROM occupancy WHERE expires<=?', Date.now());
      if (this.ctx.storage.sql.exec('SELECT slot FROM occupancy').toArray().length) return false;
      this.ctx.storage.sql.exec('INSERT INTO occupancy VALUES (1,?,?)', reservation, expires);
      return true;
    });
    await this.ctx.storage.sync();
    return accepted;
  }
  async renew(reservation: string): Promise<number> {
    const expires = Date.now() + 120000;
    const changed = this.ctx.storage.sql
      .exec(
        'UPDATE occupancy SET expires=? WHERE reservation=? AND expires>? RETURNING slot',
        expires,
        reservation,
        Date.now(),
      )
      .toArray().length;
    await this.ctx.storage.sync();
    return changed ? expires : 0;
  }
  async release(reservation: string): Promise<void> {
    this.ctx.storage.sql.exec('DELETE FROM occupancy WHERE reservation=?', reservation);
    await this.ctx.storage.sync();
  }
}
