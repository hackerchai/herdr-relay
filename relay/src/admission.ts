import { DurableObject } from 'cloudflare:workers';
import { AdmissionLedger, type LeaseKind, type Decision, type Usage } from './admission-ledger.ts';
import { ProtocolError } from './protocol.ts';
import type { Env } from './index.ts';

// One stable named object for the entire deployment. Never include the IP,
// device, origin, code version or date in this name: doing so resets quotas.
export const ADMISSION_OBJECT = 'global-admission-v1';
export function admitted(result: Decision): Decision {
  if (result.code)
    throw new ProtocolError(
      result.code,
      result.code === 'RELAY_BUSY'
        ? 503
        : result.code === 'CHANNEL_EXPIRED' || result.code === 'CHANNEL_CONFLICT'
          ? 409
          : 429,
      result.retry_after,
    );
  return result;
}
export class Admission extends DurableObject<Env> {
  private ledger: AdmissionLedger;
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    this.ledger = new AdmissionLedger(ctx.storage, env);
  }
  async begin(ip: string): Promise<Decision> {
    const r = this.ledger.begin(ip);
    await this.arm();
    return r;
  }
  finish(id: string, status: number, calls = 0): void {
    this.ledger.finish(id, status, calls);
  }
  async hold(
    kind: LeaseKind,
    id: string,
    owner: string,
    duration: number,
    diagnostic = false,
    usage?: Usage,
  ): Promise<Decision> {
    if (this.env.ADMISSION_ENABLED !== 'true' && !(kind === 'challenge' && diagnostic)) {
      this.ledger.charge(usage);
      return { code: 'RELAY_BUSY', retry_after: 60 };
    }
    const r = this.ledger.hold(kind, id, owner, duration, usage);
    await this.arm();
    return r;
  }
  renew(id: string, owner: string, usage?: Usage): Decision {
    if (this.env.ADMISSION_ENABLED === 'true') return this.ledger.renew(id, owner, usage);
    this.ledger.charge(usage);
    return { code: 'RELAY_BUSY', retry_after: 60 };
  }
  authenticated(id: string, owner: string, usage?: Usage): void {
    this.ledger.charge(usage);
    this.ledger.authenticated(id, owner);
  }
  release(id: string, owner: string, usage?: Usage): void {
    this.ledger.release(id, owner, usage);
  }
  status() {
    this.ledger.charge();
    return { ...this.ledger.status(), admission_enabled: this.env.ADMISSION_ENABLED === 'true' };
  }
  async alarm(): Promise<void> {
    this.ledger.charge();
    this.ledger.cleanup();
    await this.arm();
  }
  private async arm(): Promise<void> {
    const deadline = this.ledger.nextDeadline(),
      current = await this.ctx.storage.getAlarm();
    if (deadline !== null) {
      const next = Math.max(Date.now() + 1000, deadline);
      if (current === null || next < current) await this.ctx.storage.setAlarm(next);
    } else if (current !== null) await this.ctx.storage.deleteAlarm();
  }
}
