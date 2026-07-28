/// The CLI host.
///
/// It implements the `ABI.md` envelope against the evaluator's step machine
/// and nothing else. The evaluator cannot tell this host from the extension
/// host, which is the point: a CLI test is evidence about the browser runtime
/// rather than a separate thing that resembles it.
///
/// Host obligations, all discharged here: refuse any unregistered tool,
/// register no final-submit capability, validate both directions against the
/// JTDs, store blobs itself, and keep an append-only log ordered by sequence.
import { readFile } from "node:fs/promises";
import { validateAgainst } from "../tools/generated/index.mjs";
import { buildCatalogue } from "../tools/catalogue.mjs";
import { ArtifactStore } from "./artifacts.mjs";
import { assertBindingAllowed } from "./provider.mjs";

const REQUEST_SCHEMA = "agent-dsl://schemas/envelope/tool-request/1.0.0";

/// An append-only record ordered by sequence. Never truncated: every step is
/// evidence.
export class RunLog {
  entries = [];

  append(kind, detail) {
    this.entries.push({ kind, ...detail });
  }

  checkpoint(call, label, data) {
    this.append("checkpoint", {
      sequence: call.sequence,
      runId: call.runId,
      label,
      data,
    });
  }

  get checkpoints() {
    return this.entries.filter((e) => e.kind === "checkpoint");
  }
}

/// Load the evaluator wasm and wrap its exports in the step loop.
export async function instantiate(wasmPath) {
  const bytes = await readFile(wasmPath);
  const { instance } = await WebAssembly.instantiate(bytes, {});
  const exports = instance.exports;
  for (const name of ["tb_alloc", "tb_dealloc", "tb_start", "tb_step", "tb_last_error", "memory"]) {
    if (!(name in exports)) throw new Error(`evaluator wasm is missing export "${name}"`);
  }

  const encoder = new TextEncoder();
  const decoder = new TextDecoder();

  const write = (text) => {
    const encoded = encoder.encode(text);
    const ptr = exports.tb_alloc(encoded.length);
    new Uint8Array(exports.memory.buffer, ptr, encoded.length).set(encoded);
    return { ptr, len: encoded.length };
  };

  const readPacked = (packed) => {
    const ptr = Number(BigInt.asUintN(64, packed) >> 32n);
    const len = Number(BigInt.asUintN(64, packed) & 0xffffffffn);
    if (len === 0) return null;
    return decoder.decode(new Uint8Array(exports.memory.buffer, ptr, len));
  };

  const lastError = () => readPacked(exports.tb_last_error()) ?? "(no error message)";

  return {
    start(ir, config) {
      const irBuffer = write(ir);
      const cfgBuffer = write(config);
      const code = exports.tb_start(irBuffer.ptr, irBuffer.len, cfgBuffer.ptr, cfgBuffer.len);
      exports.tb_dealloc(irBuffer.ptr, irBuffer.len);
      exports.tb_dealloc(cfgBuffer.ptr, cfgBuffer.len);
      if (code !== 0) throw new Error(`evaluator refused the graph: ${lastError()}`);
    },
    step(response) {
      let buffer = { ptr: 0, len: 0 };
      if (response !== undefined && response !== null) buffer = write(JSON.stringify(response));
      const packed = exports.tb_step(buffer.ptr, buffer.len);
      if (buffer.len > 0) exports.tb_dealloc(buffer.ptr, buffer.len);
      const text = readPacked(packed);
      if (text === null) throw new Error(`evaluator step failed: ${lastError()}`);
      return JSON.parse(text);
    },
  };
}

/// Dispatch one `tool.request`, per `ABI.md` — Dispatch. Steps 2 to 4 produce
/// a *dispatch denial*, which is a different thing from the tool running and
/// failing, and the evaluator treats it differently.
async function dispatch(request, { catalogue, artifacts, log, policy, provider }) {
  const problems = validateAgainst(REQUEST_SCHEMA, request);
  if (problems.length > 0) {
    return denial(request, `envelope is malformed: ${problems.join("; ")}`);
  }

  const body = request.body ?? {};

  // A model call is dispatched through the provider rather than the tool
  // registry: `model.invoke` is a built-in node, and which model answers it is
  // configuration. The residency gate runs before anything leaves this
  // process.
  if (request.tool === "model.invoke") {
    try {
      assertBindingAllowed(body.binding);
      const response = await provider.invoke({
        binding: body.binding,
        prompt: body.prompt,
        outputSchemaUri: body.outputSchemaUri,
        input: body.input,
      });
      log.append("model", {
        sequence: request.sequence,
        nodeId: request.nodeId,
        binding: body.binding,
      });
      return success(request, { result: response });
    } catch (cause) {
      return failure(request, "PROVIDER_FAILED", cause.message);
    }
  }

  const tool = catalogue.get(request.tool);
  if (!tool) {
    const known = [...catalogue.keys()].sort().join(", ");
    return denial(
      request,
      `tool "${request.tool}" is not in the registry; register a tool before a graph may name it. Registered: ${known}`,
    );
  }

  if (tool.inputSchemaUri) {
    const inputProblems = validateAgainst(tool.inputSchemaUri, body);
    if (inputProblems.length > 0) {
      return denial(request, `input does not satisfy ${tool.inputSchemaUri}: ${inputProblems.join("; ")}`);
    }
  }

  let produced;
  try {
    produced = await tool.run(body, request, { policy, log, artifacts });
  } catch (cause) {
    return failure(request, "TOOL_FAILED", cause.message);
  }

  if (tool.resultSchemaUri) {
    const resultProblems = validateAgainst(tool.resultSchemaUri, produced.result ?? {});
    if (resultProblems.length > 0) {
      return failure(
        request,
        "RESULT_SCHEMA_VIOLATION",
        `result does not satisfy ${tool.resultSchemaUri}: ${resultProblems.join("; ")}`,
      );
    }
  }

  log.append("tool", {
    sequence: request.sequence,
    nodeId: request.nodeId,
    tool: request.tool,
    callId: request.id,
    files: (produced.files?.items ?? []).map((f) => f.artifactId),
  });

  return success(request, produced);
}

function success(request, produced) {
  const body = { ...produced.result ? { result: produced.result } : {}, ...produced };
  return { v: 1, kind: "tool.response", id: request.id, ok: true, body };
}

function denial(request, message) {
  return {
    v: 1,
    kind: "tool.response",
    id: request.id,
    ok: false,
    error: { code: "DISPATCH_DENIED", retryable: false, message },
  };
}

function failure(request, code, message) {
  return {
    v: 1,
    kind: "tool.response",
    id: request.id,
    ok: false,
    error: { code, retryable: true, message },
  };
}

/// Drive a graph to completion. Returns the terminal envelope plus everything
/// worth inspecting afterwards.
export async function run({
  wasmPath,
  ir,
  inputs,
  baseUrl,
  provider,
  policy = { autoApprove: ["form-write", "navigate"] },
  instanceId = "inst_0001",
  runId = "run_0001",
  maxSteps = 10_000,
}) {
  const artifacts = new ArtifactStore({ root: null, instanceId });
  const log = new RunLog();
  const catalogue = buildCatalogue({ baseUrl, artifacts });

  // The registry is the capability surface. This assertion is a standing
  // guard, not a comment: nothing may register a way to submit.
  for (const name of catalogue.keys()) {
    if (/submit/i.test(name)) {
      throw new Error(`tool "${name}" looks like a submit capability; none may be registered`);
    }
  }

  const evaluator = await instantiate(wasmPath);
  evaluator.start(ir, JSON.stringify({ instanceId, runId, inputs }));

  const requests = [];
  let response;
  for (let i = 0; i < maxSteps; i += 1) {
    const envelope = evaluator.step(response);

    if (envelope.kind === "run.finished") {
      return { finished: envelope, requests, log, artifacts };
    }
    if (envelope.kind !== "tool.request") {
      throw new Error(`evaluator produced an unknown envelope kind "${envelope.kind}"`);
    }

    requests.push(envelope);
    response = await dispatch(envelope, { catalogue, artifacts, log, policy, provider });
  }
  throw new Error(`run did not terminate within ${maxSteps} steps`);
}
