import { ProtocolError } from './protocol.ts';

/** Legacy window and rate cap for endpoints that did not negotiate stream v2. */
export const WINDOW = 65536;
export const LEGACY_BYTES_PER_SECOND = 262144;
export const MAX_BYTES_PER_SECOND = 4194304;
export const MAX_WINDOW = 1048576;
export const MESSAGE_RATE = 150;
export const MESSAGE_BURST = 300;
/** A present `window` means stream v2: that window and a separate credit budget. */
export interface Limits {
  bytes_per_second: number;
  window?: number;
}
export function windowOf(limits: Limits): number {
  return limits.window ?? WINDOW;
}
/** Both endpoints of a channel must receive the same limits. */
export function channelLimits(stream: Required<Limits>, v2: boolean): Limits {
  return v2
    ? { bytes_per_second: stream.bytes_per_second, window: stream.window }
    : { bytes_per_second: Math.min(stream.bytes_per_second, LEGACY_BYTES_PER_SECOND) };
}
export interface Quotas {
  attach: number;
  short: number;
  pending: number;
  stream: Required<Limits>;
}
export function quotas(env: {
  MAX_DEVICE_ATTACH?: string;
  MAX_DEVICE_SHORT?: string;
  MAX_PENDING_CHANNELS?: string;
  CHANNEL_BYTES_PER_SECOND?: string;
  CHANNEL_WINDOW_BYTES?: string;
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
    attach: integer(env.MAX_DEVICE_ATTACH, 8, 1, 64),
    short: integer(env.MAX_DEVICE_SHORT, 4, 1, 32),
    pending: integer(env.MAX_PENDING_CHANNELS, 4, 1, 32),
    stream: {
      bytes_per_second: integer(env.CHANNEL_BYTES_PER_SECOND, 1048576, 16384, MAX_BYTES_PER_SECOND),
      window: integer(env.CHANNEL_WINDOW_BYTES, 262144, WINDOW, MAX_WINDOW),
    },
  };
}

export interface Flow {
  sent: string;
  ack: string;
  progress: number;
  peak: number;
  /** `credits` exists only for stream v2 flows. */
  budget: { at: number; bytes: number; messages: number; credits?: number };
}
export function burstOf(limits: Limits): number {
  return Math.max(windowOf(limits), limits.bytes_per_second * 2);
}
export function initialFlow(now: number, limits: Limits): Flow {
  return {
    sent: '0',
    ack: '0',
    progress: now,
    peak: 0,
    budget: {
      at: now,
      bytes: burstOf(limits),
      messages: MESSAGE_BURST,
      ...(limits.window === undefined ? {} : { credits: MESSAGE_BURST }),
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
  const refill = (n: number) => Math.min(MESSAGE_BURST, n + elapsed * MESSAGE_RATE);
  const budget: Flow['budget'] = {
    at: Math.max(now, state.budget.at),
    bytes: Math.min(burstOf(limits), state.budget.bytes + elapsed * limits.bytes_per_second),
    messages: refill(state.budget.messages),
  };
  // Stream v2 charges credits to their own bucket, so heavy traffic in one
  // direction cannot starve the acknowledgements the other direction needs.
  const separate = limits.window !== undefined;
  if (separate) budget.credits = refill(state.budget.credits ?? MESSAGE_BURST);
  const bucket = separate && frame.kind === 2 ? 'credits' : 'messages';
  if (budget[bucket]! < 1 || budget.bytes < frame.payload.length)
    throw new ProtocolError('RATE_LIMITED', 429);
  budget[bucket]! -= 1;
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
      inflight > BigInt(windowOf(limits)) ||
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
