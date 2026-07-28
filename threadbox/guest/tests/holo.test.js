/// Unit tests for the Holo coordinate algebra in `assembly/holo.ts`.
/// Pure math, no I/O — these compile a tiny entry re-exporting holo.ts
/// to WASM and exercise the packed-i64 return shape end to end.
import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { readFileSync, writeFileSync, copyFileSync, mkdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

const GUEST_DIR = process.cwd();
const ASC = join(GUEST_DIR, 'node_modules/.bin/asc');

function buildHolo() {
  const dir = join(tmpdir(), `holo-guest-${randomUUID()}`);
  mkdirSync(join(dir, 'assembly'), { recursive: true });
  copyFileSync(join(GUEST_DIR, 'assembly/holo.ts'), join(dir, 'assembly/holo.ts'));
  const entry = join(dir, 'assembly', 'entry.ts');
  writeFileSync(entry, `export * from "./holo";\n`);
  const out = join(dir, 'holo.wasm');
  execFileSync(ASC, [entry, '-o', out, '--exportRuntime'], { cwd: dir });
  return readFileSync(out);
}

test('normToSentPixel maps 0 and 1000 to the endpoints', async () => {
  const wasm = buildHolo();
  const { instance } = await WebAssembly.instantiate(wasm, {
    env: { abort: () => { throw new Error('abort'); } },
  });
  const f = instance.exports.normToSentPixel;
  assert.strictEqual(f(0, 428), 0);
  assert.strictEqual(f(1000, 428), 428);
  assert.strictEqual(f(500, 428), 214);
});

test('normToOriginalPixel inverts a no-scale, no-pad send', async () => {
  const wasm = buildHolo();
  const { instance } = await WebAssembly.instantiate(wasm, {
    env: { abort: () => { throw new Error('abort'); } },
  });
  const e = instance.exports;
  // Original 428x506, sent as 428x506 (no scale, no pad). Holo's
  // normalized (100,126) must map back to ~(43,64) — the pixel the
  // live HAI call returned for the search box.
  const packed = e.normToOriginalPixel(100, 126, 428, 506, 428, 506, 0, 0, 428, 506);
  assert.strictEqual(e.unpackX(packed), 43);
  assert.strictEqual(e.unpackY(packed), 64);
});

test('normToOriginalPixel inverts a scale to 1280 wide with right/bottom pad', async () => {
  const wasm = buildHolo();
  const { instance } = await WebAssembly.instantiate(wasm, {
    env: { abort: () => { throw new Error('abort'); } },
  });
  const e = instance.exports;
  const plan = e.planScale(428, 506, 1280, 1600);
  const contentWidth = e.unpackX(plan);
  const contentHeight = e.unpackY(plan);
  assert.strictEqual(contentWidth, 1280);
  // 506*1280/428 = 1513.08..., integer-rounded to 1513.
  assert.ok(contentHeight >= 1512 && contentHeight <= 1514, `contentHeight=${contentHeight}`);

  // Search box at original (43,64). Forward map: scale to contentWidth
  // wide -> (43*contentWidth/428, 64*contentHeight/506). Normalize to
  // [0,1000] against the 1280x1600 canvas. Round-trip back must land
  // within a couple of pixels of (43,64).
  const fwdX = Math.round(43 * contentWidth / 428);
  const fwdY = Math.round(64 * contentHeight / 506);
  const normX = Math.round(fwdX / 1280 * 1000);
  const normY = Math.round(fwdY / 1600 * 1000);
  const packed = e.normToOriginalPixel(normX, normY, 1280, 1600, contentWidth, contentHeight, 0, 0, 428, 506);
  const x = e.unpackX(packed);
  const y = e.unpackY(packed);
  assert.ok(Math.abs(x - 43) <= 2, `x=${x}`);
  assert.ok(Math.abs(y - 64) <= 2, `y=${y}`);
});

test('normToOriginalPixel returns -1 for a coordinate in the pad', async () => {
  const wasm = buildHolo();
  const { instance } = await WebAssembly.instantiate(wasm, {
    env: { abort: () => { throw new Error('abort'); } },
  });
  const e = instance.exports;
  // Canvas 1280x1600, content 1280x1514 at (0,0). y=1000 -> 1600 (pad).
  assert.strictEqual(e.normToOriginalPixel(0, 1000, 1280, 1600, 1280, 1514, 0, 0, 428, 506), -1n);
});

test('planScale preserves aspect ratio', async () => {
  const wasm = buildHolo();
  const { instance } = await WebAssembly.instantiate(wasm, {
    env: { abort: () => { throw new Error('abort'); } },
  });
  const e = instance.exports;
  const plan = e.planScale(428, 506, 1280, 1600);
  const cw = e.unpackX(plan);
  const ch = e.unpackY(plan);
  // 428/506 == cw/ch within integer-rounding noise (±1px).
  assert.ok(Math.abs(cw * 506 - ch * 428) <= 506, `aspect not preserved: ${cw}x${ch}`);
});
