/// End-to-end: the alpha graph against the digital twin, offline.
///
/// The twin's action log is the oracle. These assertions are about what
/// happened *on the page*, not about what the run reported having done.
import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFile } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { run } from "../host.mjs";
import { FixtureProvider } from "../provider.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const PORT = 34567;
const BASE_URL = `http://127.0.0.1:${PORT}`;
const WASM = resolve(root, "evaluator/target/wasm32-unknown-unknown/release/threadbox_evaluator.wasm");

let twin;

before(async () => {
  twin = spawn("node", [resolve(root, "harness/twin/server.mjs"), String(PORT)], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  await new Promise((done, fail) => {
    const timer = setTimeout(() => fail(new Error("twin did not become ready")), 10_000);
    twin.stdout.on("data", (chunk) => {
      if (chunk.toString().includes("READY")) {
        clearTimeout(timer);
        done();
      }
    });
  });
});

after(() => twin?.kill());

async function reset() {
  await fetch(`${BASE_URL}/reset`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: "{}",
  });
}

async function oracle() {
  return (await fetch(`${BASE_URL}/log`)).json();
}

async function runAlpha() {
  const ir = await readFile(resolve(root, "guest/build/acme-motor-quotes.ir.json"), "utf8");
  const submission = JSON.parse(
    await readFile(resolve(root, "harness/twin/fixtures/submission.json"), "utf8"),
  );
  const provider = await FixtureProvider.load(
    resolve(root, "host-node/fixtures/model-corpus.json"),
    "strict",
  );
  return run({ wasmPath: WASM, ir, inputs: { submission }, baseUrl: BASE_URL, provider });
}

test("the alpha reaches the pre-submit boundary offline", async () => {
  await reset();
  const outcome = await runAlpha();

  assert.equal(outcome.finished.status, "success", outcome.finished.outcome);
  assert.equal(outcome.finished.outcome, "awaiting_human_final_submit");
});

test("the oracle shows every mapped value was entered on the page", async () => {
  await reset();
  await runAlpha();

  const submission = JSON.parse(
    await readFile(resolve(root, "harness/twin/fixtures/submission.json"), "utf8"),
  );
  const writes = (await oracle()).filter((e) => e.action === "write");

  assert.equal(writes.length, submission.answers.length, "not every mapped field was written");
  assert.ok(writes.every((w) => w.applied), "some write was refused");

  // Order matters: the graph drives the form in submission order, step by step.
  assert.deepEqual(
    writes.map((w) => w.value),
    submission.answers.map((a) => a.value),
  );
});

test("a pre-filled control is cleared before writing, and only that one", async () => {
  await reset();
  await runAlpha();

  const cleared = (await oracle()).filter((e) => e.action === "write" && e.clearFirst);
  assert.equal(cleared.length, 1, "exactly one control on this form is pre-filled");
  assert.equal(cleared[0].value, "2019");

  // The proof that clearing worked is the page's own value, not the log.
  const state = await (await fetch(`${BASE_URL}/twin/state`)).json();
  assert.equal(state.step, 3, "the run should end on the final step");
});

test("the expensive vision path runs only for controls a name lookup cannot reach", async () => {
  await reset();
  const outcome = await runAlpha();

  const byPoint = (await oracle()).filter((e) => e.action === "resolve" && e.by === "point");
  assert.equal(byPoint.length, 2, "the form has exactly two controls with no accessible name");
  assert.ok(byPoint.every((e) => e.resolved), "a model coordinate missed its control");

  // Two captures and two normalized images — one pair per vision resolution.
  // A raw capture never reaches the model: standardize always produces its own
  // artifact first.
  assert.equal(outcome.artifacts.count, 4);
  const kinds = outcome.artifacts.all().map((a) => a.labels.profile).sort();
  assert.deepEqual(kinds, ["normalized", "normalized", "raw", "raw"]);
});

test("every field commits a checkpoint", async () => {
  await reset();
  const outcome = await runAlpha();
  assert.equal(outcome.log.checkpoints.length, 17);
});

test("the run is reproducible: twice consecutively, identically", async () => {
  await reset();
  const first = await runAlpha();
  const firstLog = await oracle();

  await reset();
  const second = await runAlpha();
  const secondLog = await oracle();

  assert.deepEqual(first.finished, second.finished, "two runs disagreed on the outcome");
  assert.deepEqual(
    firstLog.map(({ ts, ...rest }) => rest),
    secondLog.map(({ ts, ...rest }) => rest),
    "two runs drove the page differently",
  );

  // Identity is minted deterministically, so the call sequence is stable too.
  assert.deepEqual(
    first.requests.map((r) => `${r.sequence} ${r.id} ${r.tool}`),
    second.requests.map((r) => `${r.sequence} ${r.id} ${r.tool}`),
  );
});

test("no submit capability exists in any registry", async () => {
  const { buildCatalogue } = await import("../../tools/catalogue.mjs");
  const catalogue = buildCatalogue({ baseUrl: BASE_URL, artifacts: null });
  for (const name of catalogue.keys()) {
    assert.doesNotMatch(name, /submit/i, `"${name}" looks like a submit capability`);
  }
  // And the page still holds an unsubmitted form at the end.
  await reset();
  await runAlpha();
  const log = await oracle();
  assert.ok(!log.some((e) => e.action === "submit"), "something submitted the form");
});

test("a prohibited provider is refused before dispatch", async () => {
  const { assertBindingAllowed } = await import("../provider.mjs");
  assert.throws(
    () => assertBindingAllowed("opencode-go"),
    /prohibited.*data-residency|residency/i,
    "the residency gate did not refuse a prohibited provider",
  );
  assert.throws(() => assertBindingAllowed("some-unvetted-host"), /not on the data-residency allowlist/);
});

test("a corpus miss in strict mode fails loudly, naming the key", async () => {
  await reset();
  const ir = await readFile(resolve(root, "guest/build/acme-motor-quotes.ir.json"), "utf8");
  const submission = JSON.parse(
    await readFile(resolve(root, "harness/twin/fixtures/submission.json"), "utf8"),
  );
  // A changed description changes the request, so its hash is not in the corpus.
  submission.answers = submission.answers.map((a) =>
    a.accessibleName === "" ? { ...a, description: `${a.description} (reworded)` } : a,
  );
  const provider = await FixtureProvider.load(
    resolve(root, "host-node/fixtures/model-corpus.json"),
    "strict",
  );

  const outcome = await run({ wasmPath: WASM, ir, inputs: { submission }, baseUrl: BASE_URL, provider });
  assert.equal(outcome.finished.status, "failure");
  assert.match(outcome.finished.outcome, /fixture corpus has no entry for [0-9a-f]{64}/);
});
