/** The public hosted relay stays open to everyone, whatever its configuration. */
export const PUBLIC_ORIGIN = 'https://herdr-relay.hackerchai.com';
export class ProtocolError extends Error {
  code: string;
  status: number;
  retryAfter?: number;
  constructor(code = 'PROTOCOL_ERROR', status = 400, retryAfter?: number) {
    super(code);
    this.code = code;
    this.status = status;
    this.retryAfter = retryAfter;
  }
}
export interface Challenge {
  version: number;
  origin: string;
  method: string;
  path: string;
  device_id: string;
  public_key: string;
  nonce: string;
  expires_at: string;
  payload_sha256: string;
}
export interface Proof {
  challenge: Challenge;
  signature: string;
}
export const encoder = new TextEncoder();
export function isHex(value: unknown, length: number): value is string {
  return typeof value === 'string' && value.length === length && /^[0-9a-f]+$/.test(value);
}
export function bytes(hex: string): Uint8Array<ArrayBuffer> {
  if (!isHex(hex, hex.length) || hex.length % 2 !== 0) throw new ProtocolError();
  return Uint8Array.from(hex.match(/../g)!.map((b) => parseInt(b, 16)));
}
export function hex(value: ArrayBuffer | Uint8Array): string {
  return Array.from(value instanceof Uint8Array ? value : new Uint8Array(value), (b) =>
    b.toString(16).padStart(2, '0'),
  ).join('');
}
export function randomHex(size = 16): string {
  return hex(crypto.getRandomValues(new Uint8Array(size)));
}
export async function digest(value: string | Uint8Array<ArrayBuffer>): Promise<string> {
  return hex(
    await crypto.subtle.digest(
      'SHA-256',
      typeof value === 'string' ? encoder.encode(value) : value,
    ),
  );
}
export function originValid(value: unknown): value is string {
  if (typeof value !== 'string' || value.length > 512) return false;
  try {
    const u = new URL(value);
    return u.protocol === 'https:' && u.origin === value && !u.username && !u.password;
  } catch {
    return false;
  }
}
export function route(
  method: string,
  path: string,
): { publisher: string; operation: string; channel?: string; role?: string } {
  if (method === 'POST' && path === '/v1/operator/status')
    return { publisher: '', operation: 'operator' };
  const match =
    /^\/v1\/devices\/([0-9a-f]{64})\/(control|invitations|pair|state|access|status|channels\/([0-9a-f]{32})\/(publisher|connector|release|authenticated))$/.exec(
      path,
    );
  if (
    !match ||
    method !==
      (['invitations', 'pair', 'state', 'access', 'status'].includes(match[2]) ||
      ['release', 'authenticated'].includes(match[4])
        ? 'POST'
        : 'GET')
  )
    throw new ProtocolError();
  return {
    publisher: match[1],
    operation: ['release', 'authenticated'].includes(match[4])
      ? match[4]
      : match[3]
        ? 'join'
        : match[2],
    channel: match[3],
    role: match[4],
  };
}
export function record(value: unknown, keys: string[]): Record<string, unknown> {
  if (
    typeof value !== 'object' ||
    value === null ||
    Array.isArray(value) ||
    Object.keys(value).length !== keys.length ||
    keys.some((k) => !Object.hasOwn(value, k))
  )
    throw new ProtocolError();
  return value as Record<string, unknown>;
}
export function strictJson(text: string): unknown {
  if (encoder.encode(text).length > 8192) throw new ProtocolError();
  try {
    const value: unknown = JSON.parse(text);
    // The wire grammar is compact JSON with literal ASCII fields, canonical
    // numbers and no whitespace. Re-encoding also rejects duplicate keys.
    if (JSON.stringify(value) !== text) throw new ProtocolError();
    return value;
  } catch {
    throw new ProtocolError();
  }
}
export function challenge(value: unknown): Challenge {
  const v = record(value, [
    'version',
    'origin',
    'method',
    'path',
    'device_id',
    'public_key',
    'nonce',
    'expires_at',
    'payload_sha256',
  ]);
  if (Number.isInteger(v.version) && v.version !== 1)
    throw new ProtocolError('INCOMPATIBLE_PROTOCOL', 400);
  if (
    v.version !== 1 ||
    !originValid(v.origin) ||
    typeof v.method !== 'string' ||
    typeof v.path !== 'string' ||
    !isHex(v.device_id, 64) ||
    !isHex(v.public_key, 64) ||
    !isHex(v.nonce, 32) ||
    !isHex(v.payload_sha256, 64) ||
    typeof v.expires_at !== 'string' ||
    !/^[1-9][0-9]{0,15}$/.test(v.expires_at) ||
    !Number.isSafeInteger(Number(v.expires_at))
  )
    throw new ProtocolError();
  route(v.method, v.path);
  return v as unknown as Challenge;
}
export function canonicalAccess(input: Challenge): string {
  const c = challenge(input);
  return [
    'herdr-relay-access-v1',
    c.origin,
    c.method,
    c.path,
    c.device_id,
    c.nonce,
    c.expires_at,
    c.payload_sha256,
    '',
  ].join('\n');
}
export async function verifyAccess(proof: Proof, origin: string, now: number): Promise<boolean> {
  try {
    const c = challenge(proof.challenge);
    const expiration = Number(c.expires_at);
    if (
      c.origin !== origin ||
      expiration <= now ||
      expiration > now + 60 ||
      !isHex(proof.signature, 128)
    )
      return false;
    // RFC 8410 Ed25519 SubjectPublicKeyInfo, no parameters.
    if ((await digest(bytes('302a300506032b6570032100' + c.public_key))) !== c.device_id)
      return false;
    const key = await crypto.subtle.importKey(
      'raw',
      bytes(c.public_key),
      { name: 'Ed25519' },
      false,
      ['verify'],
    );
    return await crypto.subtle.verify(
      'Ed25519',
      key,
      bytes(proof.signature),
      encoder.encode(canonicalAccess(c)),
    );
  } catch {
    return false;
  }
}
export async function boundedBody(request: Request): Promise<string> {
  if (!request.body) return '';
  const reader = request.body.getReader();
  const data = new Uint8Array(8192);
  let expired = false;
  const timer = setTimeout(() => {
    expired = true;
    void reader.cancel().catch(() => {});
  }, 10000);
  let size = 0;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (expired) throw new ProtocolError('REQUEST_TIMEOUT', 408);
      if (done) break;
      size += value.byteLength;
      if (size > 8192) {
        await reader.cancel();
        throw new ProtocolError('MESSAGE_TOO_LARGE', 413);
      }
      data.set(value, size - value.byteLength);
    }
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(
      data.subarray(0, size),
    );
  } catch (error) {
    if (expired) throw new ProtocolError('REQUEST_TIMEOUT', 408);
    throw error;
  } finally {
    clearTimeout(timer);
    reader.releaseLock();
  }
}
export function failure(error: unknown): Response {
  const known = error instanceof ProtocolError;
  const headers: Record<string, string> = { 'Cache-Control': 'no-store' };
  if (known && error.status === 429) headers['Retry-After'] = '60';
  if (known && error.status === 503) headers['Retry-After'] = '5';
  if (known && error.retryAfter) headers['Retry-After'] = String(error.retryAfter);
  if (known && error.code.startsWith('RELAY_AUTH_'))
    headers['WWW-Authenticate'] = 'Bearer realm="herdr-relay"';
  return Response.json(
    { code: known ? error.code : 'INTERNAL_ERROR' },
    { status: known ? error.status : 500, headers },
  );
}
