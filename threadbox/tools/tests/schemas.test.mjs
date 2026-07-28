/// The generated validators are a build artifact, so the thing worth testing
/// is that codegen produced working validators for the URIs the ABI names —
/// not the generator itself, which has its own suite upstream.
import { test } from "node:test";
import assert from "node:assert/strict";
import { VALIDATORS, validateAgainst } from "../generated/index.mjs";

const SUBMISSION = "agent-dsl://schemas/submission/acme-motor-quotes/0.1.0";

test("every schema URI the ABI names is registered", () => {
  for (const uri of [
    "agent-dsl://schemas/envelope/tool-request/1.0.0",
    "agent-dsl://schemas/envelope/tool-response/1.0.0",
    "agent-dsl://schemas/envelope/run-finished/1.0.0",
    "agent-dsl://schemas/fs/artifact/1.0.0",
    SUBMISSION,
  ]) {
    assert.ok(VALIDATORS.has(uri), `missing validator for ${uri}`);
  }
});

test("an unregistered URI is an error, never a silent pass", () => {
  const problems = validateAgainst("agent-dsl://schemas/nope/0.0.0", {});
  assert.equal(problems.length, 1);
  assert.match(problems[0], /is not registered/);
});

test("a well-formed tool request validates", () => {
  const problems = validateAgainst("agent-dsl://schemas/envelope/tool-request/1.0.0", {
    v: 1, kind: "tool.request", id: "call_000001", instanceId: "inst_0001",
    runId: "run_0001", nodeId: "observe-page", nodeInstanceId: "nodei_000001",
    attempt: 1, sequence: 1, tool: "page.observe", toolVersion: "0.1.0",
    body: { freshness: "new" },
  });
  assert.deepEqual(problems, []);
});

test("a malformed tool request names the offending path", () => {
  const problems = validateAgainst("agent-dsl://schemas/envelope/tool-request/1.0.0", {
    v: 1, kind: "tool.request", id: "call_000001", instanceId: "inst_0001",
    runId: "run_0001", nodeId: "observe-page", nodeInstanceId: "nodei_000001",
    attempt: "first", sequence: 1, tool: "page.observe", toolVersion: "0.1.0", body: {},
  });
  assert.equal(problems.length, 1);
  assert.match(problems[0], /\/attempt/);
});

test("the alpha submission fixture conforms to its schema", async () => {
  const { readFile } = await import("node:fs/promises");
  const doc = JSON.parse(
    await readFile(new URL("../../harness/twin/fixtures/submission.json", import.meta.url), "utf8"),
  );
  assert.deepEqual(validateAgainst(SUBMISSION, doc), []);
});
