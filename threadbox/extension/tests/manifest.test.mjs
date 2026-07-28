import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";

test("extension is a module-based Manifest V3 package", async () => {
  const manifest = JSON.parse(
    await readFile(new URL("../manifest.json", import.meta.url), "utf8"),
  );
  assert.equal(manifest.manifest_version, 3);
  assert.deepEqual(manifest.background, {
    service_worker: "background.mjs",
    type: "module",
  });
  assert.match(
    manifest.content_security_policy.extension_pages,
    /wasm-unsafe-eval/,
  );
  assert.equal(manifest.permissions, undefined);
  assert.equal(manifest.host_permissions, undefined);
});
