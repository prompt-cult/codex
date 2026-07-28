/// `agent-dsl-fs` for the CLI host.
///
/// An immutable, instance-scoped artifact namespace. **Blob bytes never cross
/// the ABI**: a tool writes bytes here and returns metadata, and the evaluator
/// reasons over that metadata without ever seeing a byte. That is what keeps a
/// graph cheap — a node passes a reference, not a payload.
///
/// Artifacts are create-only. A loop iteration that captures again creates a
/// new artifact; nothing is ever replaced.
import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

export class ArtifactStore {
  #byId = new Map();
  #bytes = new Map();
  #counter = 0;
  #root;
  #instanceId;

  constructor({ root, instanceId }) {
    this.#root = root;
    this.#instanceId = instanceId;
  }

  /// Create an artifact. `call` supplies the causal identity that makes the
  /// logical path unique, so two iterations of the same node cannot collide.
  async create(call, { name, mediaType, semanticKind, bytes, labels = {} }) {
    this.#counter += 1;
    const artifactId = `art_${String(this.#counter).padStart(6, "0")}`;
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    const logicalPath =
      `runs/${call.runId}/nodes/${call.nodeInstanceId}/calls/${call.id}/outputs/${name}`;

    const metadata = {
      schemaUri: "agent-dsl://schemas/fs/artifact/1.0.0",
      artifactId,
      blobId: `sha256:${sha256}`,
      instanceId: this.#instanceId,
      runId: call.runId,
      nodeInstanceId: call.nodeInstanceId,
      callId: call.id,
      logicalPath,
      name,
      mediaType,
      semanticKind,
      size: bytes.length,
      sha256,
      readOnly: true,
      labels,
    };

    if (this.#byId.has(artifactId)) {
      throw new Error(`artifact ${artifactId} already exists; artifacts are create-only`);
    }
    this.#byId.set(artifactId, metadata);
    this.#bytes.set(artifactId, bytes);

    if (this.#root) {
      const dir = join(this.#root, "blobs");
      await mkdir(dir, { recursive: true });
      await writeFile(join(dir, `${artifactId}-${name}`), bytes);
    }

    // What crosses the ABI is metadata only — note the absence of `bytes`.
    return {
      artifactId,
      blobId: metadata.blobId,
      name,
      mediaType,
      semanticKind,
      size: bytes.length,
      sha256,
      labels,
    };
  }

  get(artifactId) {
    return this.#byId.get(artifactId) ?? null;
  }

  bytes(artifactId) {
    const bytes = this.#bytes.get(artifactId);
    if (!bytes) throw new Error(`no bytes held for artifact "${artifactId}"`);
    return bytes;
  }

  get count() {
    return this.#byId.size;
  }

  all() {
    return [...this.#byId.values()];
  }
}
