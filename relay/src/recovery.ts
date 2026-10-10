import { isHex } from './protocol.ts';
import { MAX_WINDOW, WINDOW, burstOf, quotas, type Flow, type Limits } from './limits.ts';

export interface RestoredChannel {
  id: string;
  reservation: string;
  publisher: string;
  connector: string;
  boot: string;
  host_used: number;
  client_used: number;
  expires: number;
  purpose: string;
  rate: number;
  /** Zero for a legacy channel; otherwise the negotiated stream v2 window. */
  window?: number;
}
function rowLimits(row: RestoredChannel): Limits {
  return row.window
    ? { bytes_per_second: row.rate, window: row.window }
    : { bytes_per_second: row.rate };
}
function flowValid(value: unknown, limits: Limits): value is Flow {
  const window = limits.window ?? WINDOW;
  if (!value || typeof value !== 'object') return false;
  const f = value as Flow;
  const offset = (n: unknown) =>
    typeof n === 'string' && /^(0|[1-9][0-9]{0,19})$/.test(n) && BigInt(n) <= 0xffffffffffffffffn;
  return (
    offset(f.sent) &&
    offset(f.ack) &&
    Number.isSafeInteger(f.progress) &&
    f.progress >= 0 &&
    Number.isInteger(f.peak) &&
    f.peak >= 0 &&
    f.peak <= window &&
    !!f.budget &&
    Number.isSafeInteger(f.budget.at) &&
    f.budget.at >= 0 &&
    Number.isFinite(f.budget.bytes) &&
    f.budget.bytes >= 0 &&
    f.budget.bytes <= burstOf(limits) &&
    Number.isFinite(f.budget.messages) &&
    f.budget.messages >= 0 &&
    f.budget.messages <= 300 &&
    (limits.window === undefined
      ? f.budget.credits === undefined
      : Number.isFinite(f.budget.credits) && f.budget.credits! >= 0 && f.budget.credits! <= 300)
  );
}
/** A missing consumed role must never be recreated from its SQL ticket bit. */
export function restoredChannelValid(
  row: RestoredChannel,
  attachments: unknown[],
  now: number,
): boolean {
  try {
    if (
      ![row.id, row.reservation, row.boot].every((s) => isHex(s, 32)) ||
      ![row.publisher, row.connector].every((s) => isHex(s, 64)) ||
      row.expires <= now ||
      !['pair', 'control', 'attach'].includes(row.purpose)
    )
      return false;
    quotas({ CHANNEL_BYTES_PER_SECOND: String(row.rate) });
    if (
      row.window &&
      (!Number.isSafeInteger(row.window) || row.window < WINDOW || row.window > MAX_WINDOW)
    )
      return false;
    if (!row.window && row.rate > 262144) return false;
    const limits = rowLimits(row);
    if (
      ![0, 1].includes(row.host_used) ||
      ![0, 1].includes(row.client_used) ||
      attachments.length !== row.host_used + row.client_used
    )
      return false;
    const roles = new Set<string>();
    const pairs = attachments.map((value) => {
      if (!value || typeof value !== 'object') throw Error();
      const a = value as Record<string, unknown>;
      if (
        a.kind !== 'pair' ||
        a.channel !== row.id ||
        a.reservation !== row.reservation ||
        a.boot !== row.boot ||
        a.purpose !== row.purpose ||
        typeof a.role !== 'string' ||
        !['publisher', 'connector'].includes(a.role) ||
        roles.has(a.role) ||
        a.device !== (a.role === 'publisher' ? row.publisher : row.connector) ||
        !(a.role === 'publisher' ? row.host_used : row.client_used) ||
        a.ready !== (attachments.length === 2) ||
        typeof a.messages !== 'number' ||
        !Number.isSafeInteger(a.messages) ||
        a.messages < 0 ||
        !a.limits ||
        typeof a.limits !== 'object' ||
        (a.limits as Record<string, unknown>).bytes_per_second !== row.rate ||
        (a.limits as Record<string, unknown>).window !== limits.window ||
        Object.keys(a.limits).length !== (limits.window === undefined ? 1 : 2) ||
        !flowValid(a.flow, limits)
      )
        throw Error();
      roles.add(a.role);
      return { flow: a.flow };
    });
    if (pairs.length === 2) {
      for (const [a, b] of [
        [pairs[0], pairs[1]],
        [pairs[1], pairs[0]],
      ]) {
        const outstanding = BigInt(a.flow.sent) - BigInt(b.flow.ack);
        if (outstanding < 0n || outstanding > BigInt(limits.window ?? WINDOW)) return false;
      }
    }
    return true;
  } catch {
    return false;
  }
}
