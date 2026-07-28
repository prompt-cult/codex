/// Model providers.
///
/// `model.invoke` is one generic node in the graph; which model answers it is
/// configuration. This module holds the allowlist and the fixture provider
/// that makes a run deterministic and offline.
///
/// ## Data residency
///
/// A provider whose serving path is not on the allowlist is prohibited by
/// default, and the check happens *before* any dispatch. It is not overridable
/// by a graph, a prompt, or a model, because it is enforced here rather than
/// requested politely somewhere a model can read.
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";

/// Bindings permitted to serve a run. `fixture` is offline and always allowed.
/// Live bindings are absent in this milestone; adding one is a deliberate edit
/// here, reviewed against the residency policy, and never a graph change.
const ALLOWLIST = new Map([
  ["fixture", { residency: "offline", description: "content-addressed corpus, no network" }],
  ["vision.primary", { residency: "offline", description: "alias resolved to the fixture corpus" }],
]);

/// Explicitly refused, with the reason, so a misconfiguration fails loudly
/// rather than silently falling through to a default.
const PROHIBITED = new Map([
  ["opencode-go", "serving path is outside the permitted data-residency region"],
]);

export function assertBindingAllowed(binding) {
  const refusal = PROHIBITED.get(binding);
  if (refusal) {
    throw new Error(
      `provider "${binding}" is prohibited: ${refusal}; refusing to dispatch`,
    );
  }
  if (!ALLOWLIST.has(binding)) {
    const known = [...ALLOWLIST.keys()].join(", ");
    throw new Error(
      `provider "${binding}" is not on the data-residency allowlist; permitted: ${known}`,
    );
  }
}

/// The key a request is filed under.
///
/// Canonicalised: object keys sorted, and each input image reduced to its
/// artifact **hash** rather than its bytes. Keying on bytes would make the
/// corpus enormous and unreviewable; keying on the hash keeps an entry to a
/// few lines of JSON that a human can read in a diff.
export function corpusKey(request) {
  const canonical = canonicalise({
    binding: request.binding,
    prompt: request.prompt,
    outputSchemaUri: request.outputSchemaUri,
    input: stripBlobs(request.input),
  });
  return createHash("sha256").update(JSON.stringify(canonical)).digest("hex");
}

function stripBlobs(value) {
  if (Array.isArray(value)) return value.map(stripBlobs);
  if (value && typeof value === "object") {
    const out = {};
    for (const [key, inner] of Object.entries(value)) {
      // An artifact reference contributes its hash and its kind, never its
      // size or its per-run identity, so the key is stable across runs.
      if (inner && typeof inner === "object" && typeof inner.sha256 === "string") {
        out[key] = { sha256: inner.sha256, semanticKind: inner.semanticKind ?? null };
      } else {
        out[key] = stripBlobs(inner);
      }
    }
    return out;
  }
  return value;
}

function canonicalise(value) {
  if (Array.isArray(value)) return value.map(canonicalise);
  if (value && typeof value === "object") {
    const out = {};
    for (const key of Object.keys(value).sort()) out[key] = canonicalise(value[key]);
    return out;
  }
  return value;
}

export class FixtureProvider {
  #corpus;
  #path;
  #mode;
  #misses = [];

  constructor({ corpus, path, mode }) {
    this.#corpus = corpus;
    this.#path = path;
    this.#mode = mode; // "strict" | "record"
  }

  static async load(path, mode = "strict") {
    let corpus = {};
    try {
      corpus = JSON.parse(await readFile(path, "utf8"));
    } catch (cause) {
      if (cause.code !== "ENOENT") throw cause;
      if (mode === "strict") {
        throw new Error(`fixture corpus ${path} does not exist; run in record mode to create it`);
      }
    }
    return new FixtureProvider({ corpus, path, mode });
  }

  get misses() {
    return this.#misses;
  }

  /// Answer a `model.invoke`. In strict mode a miss is a hard failure naming
  /// the key, so a graph change that alters a prompt cannot silently fall back
  /// to a stale answer.
  async invoke(request) {
    assertBindingAllowed(request.binding);
    const key = corpusKey(request);
    const hit = this.#corpus[key];
    if (hit) return hit.response;

    this.#misses.push({ key, request });
    if (this.#mode === "strict") {
      throw new Error(
        `fixture corpus has no entry for ${key} ` +
          `(binding "${request.binding}", prompt "${request.prompt}"); ` +
          `re-run in record mode to add it`,
      );
    }
    throw new Error(
      `fixture corpus miss for ${key} and no live provider is configured in this milestone`,
    );
  }

  /// Append recorded pairs. Growing the corpus is a deliberate act that shows
  /// up as a reviewable diff, never a side effect of running the suite.
  async record(key, request, response) {
    this.#corpus[key] = {
      binding: request.binding,
      prompt: request.prompt,
      outputSchemaUri: request.outputSchemaUri,
      request: stripBlobs(request.input),
      response,
    };
    await writeFile(this.#path, `${JSON.stringify(this.#corpus, null, 2)}\n`);
  }
}
