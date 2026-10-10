import { ProtocolError, hex } from './protocol.ts';
export function decodeFrame(message: ArrayBuffer): {
  kind: number;
  channel: string;
  offset: bigint;
  payload: Uint8Array;
} {
  if (message.byteLength < 32 || message.byteLength > 16416) throw new ProtocolError();
  const view = new DataView(message);
  const kind = view.getUint8(1);
  const size = view.getUint32(28);
  if (
    view.getUint8(0) !== 1 ||
    (kind !== 1 && kind !== 2) ||
    view.getUint16(2) !== 0 ||
    size !== message.byteLength - 32 ||
    (kind === 2 && size !== 0)
  )
    throw new ProtocolError();
  return {
    kind,
    channel: hex(new Uint8Array(message, 4, 16)),
    offset: view.getBigUint64(20),
    payload: new Uint8Array(message, 32),
  };
}
