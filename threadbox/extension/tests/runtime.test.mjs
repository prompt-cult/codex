import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import {
  instantiateWasmBytes,
  requireExports,
} from "../runtime.mjs";

const wasmPath = fileURLToPath(new URL("../wasm/coordinates.wasm", import.meta.url));

test("the packaged browser artifact instantiates through the shared runtime", async () => {
  const bytes = await readFile(wasmPath);
  const instance = await instantiateWasmBytes(bytes);
  requireExports(instance, [
    "normToSentPixel",
    "normToOriginalPixel",
    "planScale",
    "unpackX",
    "unpackY",
  ]);

  assert.equal(instance.exports.normToSentPixel(1000, 428), 427);
  const plan = instance.exports.planScale(428, 506, 1280, 1600);
  assert.equal(instance.exports.unpackX(plan), 1280);
});

test("missing required exports fail closed", async () => {
  const bytes = await readFile(wasmPath);
  const instance = await instantiateWasmBytes(bytes);
  assert.throws(
    () => requireExports(instance, ["futureDslEntryPoint"]),
    /missing required function export "futureDslEntryPoint"/,
  );
});
