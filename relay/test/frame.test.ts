import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { decodeFrame } from '../src/frame.ts';
import { bytes, hex } from '../src/protocol.ts';
const fixture = JSON.parse(
  readFileSync(new URL('../../protocol/fixtures/frames-v1.json', import.meta.url), 'utf8'),
);
test('shared frame fixtures preserve exact offsets and enforce envelope bounds', () => {
  for (const c of fixture.valid) {
    const frame = decodeFrame(bytes(c.hex).buffer);
    assert.equal(String(frame.offset), c.offset, c.name);
    assert.equal(frame.kind, c.kind, c.name);
    assert.equal(frame.channel, c.channel, c.name);
    assert.equal(hex(frame.payload), c.payload, c.name);
  }
  for (const c of fixture.invalid) assert.throws(() => decodeFrame(bytes(c.hex).buffer), c.name);
});
