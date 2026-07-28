/// Record the fixture model corpus.
///
/// The alpha has no live provider. This script stands in a **mock model** that
/// answers the localization requests the graph makes, and files each answer
/// under the hash of its canonicalised request. After it has run, the same
/// graph replays offline in strict mode with no network and no stochasticity.
///
/// The mock is deliberately *good*: it answers with the coordinate a competent
/// vision model would return, derived from the twin's own declared geometry.
/// That keeps the alpha's subject the runtime — identity, artifacts, guards,
/// verification, convergence — rather than model accuracy, which is a separate
/// question measured by a separate harness.
///
/// Replacing this mock with a real provider is a configuration change. That it
/// *is* only a configuration change is the property the alpha exists to prove.
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { run } from "../host-node/host.mjs";
import { corpusKey, assertBindingAllowed } from "../host-node/provider.mjs";
import { newSession, visibleFields, fieldCentre } from "../harness/twin/form.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CORPUS = resolve(root, "host-node/fixtures/model-corpus.json");
const BASE_URL = process.env.TWIN_URL ?? "http://127.0.0.1:3456";

const submission = JSON.parse(
  await readFile(resolve(root, "harness/twin/fixtures/submission.json"), "utf8"),
);
const ir = await readFile(resolve(root, "guest/build/acme-motor-quotes.ir.json"), "utf8");

/// Map a target description back to the control it names, then to the point a
/// vision model would click. The twin publishes its layout, so this is the
/// ground truth a good model would approximate.
async function mockLocalize(target) {
  const answer = submission.answers.find((a) => a.description === target);
  if (!answer) throw new Error(`mock model has no ground truth for target "${target}"`);

  const state = await (await fetch(`${BASE_URL}/twin/state`)).json();
  // Rebuild the session shape the geometry helper expects from the live state.
  const session = newSession();
  session.step = state.step;
  const ordered = visibleFields(session);
  const index = ordered.findIndex((f) => f.key === answer.id);
  if (index < 0) throw new Error(`control "${answer.id}" is not on step ${state.step}`);

  const centre = fieldCentre(session, ordered[index].handle);
  return { x: centre.x, y: centre.y, confidence: 0.95, note: `mock localization of ${answer.id}` };
}

/// A provider that answers from the mock and records every pair.
class RecordingProvider {
  recorded = {};

  async invoke(request) {
    assertBindingAllowed(request.binding);
    const target = request.input?.target;
    if (typeof target !== "string") {
      throw new Error("a localization request must carry a string target");
    }
    const response = await mockLocalize(target);
    this.recorded[corpusKey(request)] = {
      binding: request.binding,
      prompt: request.prompt,
      outputSchemaUri: request.outputSchemaUri,
      target,
      response,
    };
    return response;
  }
}

await fetch(`${BASE_URL}/reset`, {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: "{}",
});

const provider = new RecordingProvider();
const outcome = await run({
  wasmPath: resolve(root, "evaluator/target/wasm32-unknown-unknown/release/threadbox_evaluator.wasm"),
  ir,
  inputs: { submission },
  baseUrl: BASE_URL,
  provider,
});

await mkdir(dirname(CORPUS), { recursive: true });
await writeFile(CORPUS, `${JSON.stringify(provider.recorded, null, 2)}\n`);

const entries = Object.keys(provider.recorded).length;
process.stderr.write(
  `recorded ${entries} model ${entries === 1 ? "answer" : "answers"} to ${CORPUS}\n` +
    `run ended: ${outcome.finished.status} — ${outcome.finished.outcome}\n`,
);
process.exit(outcome.finished.status === "success" ? 0 : 1);
