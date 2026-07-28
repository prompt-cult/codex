/// Golden-byte regression lock for `emit.ts`'s output, against the exact
/// worked fragment `IR.md` documents. `emit.ts` already works; this test
/// establishes a byte-exact baseline so a future change to `emit.ts` or
/// `ir.ts` that diverges from `IR.md`'s spelled-out example breaks
/// visibly, rather than silently.
import { test } from 'node:test';
import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

test('emit.ts golden output matches IR.md worked fragment', async () => {
  const wasmPath = join(tmpdir(), `small-graph-${randomUUID()}.wasm`);

  execFileSync(
    join(process.cwd(), 'node_modules/.bin/asc'),
    ['tests/fixtures/small-graph.ts', '-o', wasmPath, '--exportRuntime'],
    { cwd: process.cwd() }
  );

  const wasmBytes = readFileSync(wasmPath);
  let captured = Buffer.alloc(0);
  const { instance } = await WebAssembly.instantiate(wasmBytes, {
    'threadbox.ir.v1': {
      emit: (ptr, len) => {
        const mem = new Uint8Array(instance.exports.memory.buffer);
        captured = Buffer.from(mem.slice(ptr, ptr + len));
      },
    },
    env: {
      abort: (msg, file, line, column) => {
        throw new Error(`guest called abort at line ${line}, column ${column}`);
      },
    },
  });
  instance.exports.main();

  const actual = captured.toString('utf8');
  const expected = readFileSync('tests/fixtures/small-graph.json', 'utf8');
  assert.strictEqual(actual, expected);
});
