// Adapters for the external Cloudflare runtime; application objects and their
// SQL execute unchanged. These tests do not emulate admission/retry decisions.
import { registerHooks } from 'node:module';
import { DatabaseSync } from 'node:sqlite';

registerHooks({
  resolve(specifier, context, next) {
    if (specifier === 'cloudflare:workers')
      return {
        url:
          'data:text/javascript,' +
          encodeURIComponent(
            'export class DurableObject { constructor(ctx, env) { this.ctx = ctx; this.env = env; } }',
          ),
        shortCircuit: true,
      };
    return next(specifier, context);
  },
});

export function platform(sockets: TestSocket[] = []) {
  const db = new DatabaseSync(':memory:');
  let alarm: number | null = null;
  const pending: Promise<unknown>[] = [];
  const storage = {
    sql: {
      exec(query: string, ...args: (string | number)[]) {
        const rows = db.prepare(query).all(...args);
        return {
          toArray: () => rows,
          one: () => {
            if (rows.length !== 1) throw new Error('Expected one SQL row');
            return rows[0];
          },
        };
      },
    },
    transactionSync<T>(fn: () => T): T {
      db.exec('BEGIN');
      try {
        const value = fn();
        db.exec('COMMIT');
        return value;
      } catch (error) {
        db.exec('ROLLBACK');
        throw error;
      }
    },
    async getAlarm() {
      return alarm;
    },
    async setAlarm(value: number) {
      alarm = value;
    },
    async deleteAlarm() {
      alarm = null;
    },
    async sync() {},
  };
  return {
    storage,
    getWebSockets: () => sockets,
    waitUntil: (promise: Promise<unknown>) => {
      pending.push(promise);
    },
    async drain() {
      while (pending.length) await Promise.all(pending.splice(0));
    },
    async fire(object: { alarm(): Promise<void> }) {
      alarm = null;
      await object.alarm();
      await this.drain();
    },
  };
}

export class TestSocket {
  readyState = WebSocket.OPEN;
  attachment: Record<string, unknown>;
  constructor(attachment: Record<string, unknown>) {
    this.attachment = structuredClone(attachment);
  }
  deserializeAttachment() {
    return structuredClone(this.attachment);
  }
  serializeAttachment(value: Record<string, unknown>) {
    this.attachment = structuredClone(value);
  }
  close() {
    this.readyState = WebSocket.CLOSED;
  }
}
