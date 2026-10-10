import { ProtocolError } from './protocol.ts';

export const WINDOW = 65536;
export const MESSAGE_RATE = 150;
export const MESSAGE_BURST = 300;
export interface Limits {
  bytes_per_second: number;
}
export interface Quotas {
  attach: number;
  short: number;
  pending: number;
  stream: Limits;
}
export function quotas(env: {
  MAX_DEVICE_ATTACH?: string;
  MAX_DEVICE_SHORT?: string;
  MAX_PENDING_CHANNELS?: string;
  CHANNEL_BYTES_PER_SECOND?: string;
}): Quotas {
  const integer = (
    value: string | undefined,
    fallback: number,
    min: number,
    max: number,
  ): number => {
    if (value === undefined) return fallback;
    if (!/^[1-9][0-9]*$/.test(value)) throw new ProtocolError('RELAY_UNCONFIGURED', 503);
    const n = Number(value);
    if (!Number.isSafeInteger(n) || n < min || n > max)
      throw new ProtocolError('RELAY_UNCONFIGURED', 503);
    return n;
  };
  return {
    attach: integer(env.MAX_DEVICE_ATTACH, 8, 1, 8),
    short: integer(env.MAX_DEVICE_SHORT, 4, 1, 4),
    pending: integer(env.MAX_PENDING_CHANNELS, 4, 1, 4),
    stream: { bytes_per_second: integer(env.CHANNEL_BYTES_PER_SECOND, 262144, 16384, 262144) },
  };
}

export interface Flow {
  sent: string;
  ack: string;
  progress: number;
  peak: number;
  budget: { at: number; bytes: number; messages: number };
}
export function initialFlow(now: number, limits: Limits): Flow {
  return {
    sent: '0',
    ack: '0',
    progress: now,
    peak: 0,
    budget: {
      at: now,
      bytes: Math.max(WINDOW, limits.bytes_per_second * 2),
      messages: MESSAGE_BURST,
    },
  };
}
export function acceptFrame(
  state: Flow,
  peer: Flow,
  frame: { kind: number; offset: bigint; payload: Uint8Array },
  now: number,
  limits: Limits,
): { state: Flow; peer: Flow; forward: boolean } {
  const elapsed = Math.max(0, now - state.budget.at) / 1000;
  const budget = {
    at: Math.max(now, state.budget.at),
    bytes: Math.min(
      Math.max(WINDOW, limits.bytes_per_second * 2),
      state.budget.bytes + elapsed * limits.bytes_per_second,
    ),
    messages: Math.min(MESSAGE_BURST, state.budget.messages + elapsed * MESSAGE_RATE),
  };
  if (budget.messages < 1 || budget.bytes < frame.payload.length)
    throw new ProtocolError('RATE_LIMITED', 429);
  budget.messages -= 1;
  budget.bytes -= frame.payload.length;
  let next = { ...state, budget },
    other = peer,
    forward = true;
  if (frame.kind === 1) {
    const end = BigInt(state.sent) + BigInt(frame.payload.length),
      inflight = end - BigInt(peer.ack);
    if (
      frame.payload.length === 0 ||
      frame.payload.length > 16384 ||
      frame.offset !== BigInt(state.sent) ||
      end > 0xffffffffffffffffn ||
      inflight > BigInt(WINDOW) ||
      inflight < 0n
    )
      throw new ProtocolError();
    next = {
      ...next,
      sent: String(end),
      progress: state.sent === peer.ack ? now : state.progress,
      peak: Math.max(state.peak, Number(inflight)),
    };
  } else if (frame.kind === 2) {
    if (
      frame.payload.length !== 0 ||
      frame.offset < BigInt(state.ack) ||
      frame.offset > BigInt(peer.sent)
    )
      throw new ProtocolError();
    forward = frame.offset !== BigInt(state.ack);
    next = { ...next, ack: String(frame.offset) };
    if (forward) other = { ...peer, progress: now };
  } else throw new ProtocolError();
  return { state: next, peer: other, forward };
}
