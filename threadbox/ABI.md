# ABI.md — the boundary

The evaluator is a pure function of its inputs plus the responses it is given.
It has no clock, no randomness, no filesystem, no network, and no browser API.
Everything it wants, it asks the host for, through the one envelope below.

## Step machine

WebAssembly cannot block on a promise, so the evaluator is a step machine
rather than a coroutine. The host drives it.

| Export | Signature | Meaning |
|---|---|---|
| `tb_alloc` | `(len: i32) -> i32` | Allocate `len` bytes in guest memory; returns the pointer. |
| `tb_dealloc` | `(ptr: i32, len: i32) -> void` | Release a buffer obtained from `tb_alloc`. |
| `tb_start` | `(ir_ptr, ir_len, cfg_ptr, cfg_len) -> i32` | Load the IR and run configuration. `0` on success, non-zero on error. |
| `tb_step` | `(ptr: i32, len: i32) -> i64` | Advance. Input is empty on the first call, otherwise one `tool.response`. Returns a packed `(ptr << 32) \| len` naming a JSON document in guest memory. |
| `tb_last_error` | `() -> i64` | Packed pointer/length of the last error message. |

The host loop is:

```text
tb_start(ir, cfg)
loop {
  out = tb_step(pending_response)
  match out.kind {
    "tool.request"  => pending_response = dispatch(out)
    "run.finished"  => break
  }
}
```

`tb_step` is synchronous and total: it either returns a request, returns a
terminal, or reports an error. It never calls back into the host.

## Envelope

Every crossing is one JSON object. `v` is the envelope version and is `1`.

### `tool.request`

```json
{
  "v": 1,
  "kind": "tool.request",
  "id": "call_000012",
  "instanceId": "inst_0001",
  "runId": "run_0001",
  "nodeId": "resolve-postcode",
  "nodeInstanceId": "nodei_000007",
  "attempt": 1,
  "sequence": 42,
  "tool": "form.apply",
  "toolVersion": "0.1.0",
  "inputSchemaUri": "agent-dsl://schemas/tools/form-apply/input/0.1.0",
  "resultSchemaUri": "agent-dsl://schemas/tools/form-apply/result/0.1.0",
  "body": {}
}
```

### `tool.response`

Exactly one of `ok: true` with a body, or `ok: false` with an error.

```json
{ "v": 1, "kind": "tool.response", "id": "call_000012", "ok": true, "body": {} }
```

```json
{
  "v": 1,
  "kind": "tool.response",
  "id": "call_000012",
  "ok": false,
  "error": {
    "code": "STALE_OBSERVATION",
    "retryable": true,
    "requiresFreshObservation": true,
    "message": "the page navigated after the proposal was created"
  }
}
```

A response whose `id` does not match the outstanding request is a host defect
and the evaluator rejects it.

### `run.finished`

```json
{
  "v": 1,
  "kind": "run.finished",
  "status": "success",
  "outcome": "awaiting_human_final_submit",
  "sequence": 118,
  "summary": { "nodesExecuted": 47, "toolCalls": 31, "artifacts": 12 }
}
```

`status` is one of `success`, `exhausted`, `cancelled`, `failure`. These are
distinct states; a run that ran out of attempts did not succeed, and a run that
failed did not merely produce no files.

## Dispatch

1. The evaluator emits a `tool.request`.
2. The host validates the envelope against its JTD.
3. The host checks the run's capability grant and origin scope. A tool that is
   not in the registry, or not granted, is refused — the evaluator never learns
   a secret or a URL by asking.
4. The host validates `body` against the tool's input JTD.
5. The host dispatches to the registered MJS export.
6. The host records a redacted durable event.
7. The host validates the result against the tool's result JTD.
8. The host returns one `tool.response`.

A failure at any of steps 2–4 is a **dispatch denial** (`ok: false`, code
`DISPATCH_DENIED`) and is distinct from the tool running and failing. The
evaluator treats them differently: a denial is never retried.

## Identity

The evaluator mints identity deterministically, seeded from run configuration,
so that two runs of the same graph over the same inputs produce byte-identical
logs. Reproducibility is worth more here than globally unique ids.

| Field | Scope |
|---|---|
| `instanceId` | one instantiation of a DSL program |
| `runId` | one attempt to complete the task |
| `nodeId` | stable identity from the graph definition |
| `nodeInstanceId` | one execution of that node, including a loop iteration |
| `callId` | one tool or model invocation |
| `attempt` | ordinal within a named retry policy |
| `sequence` | total order over the append-only log |
| `blobId` | content hash of an artifact's bytes |

A loop executes the same `nodeId` many times and never overwrites an earlier
result. Sequence is metadata, not identity.

## `agent-dsl-fs`

A virtual, immutable, instance-scoped artifact namespace. It is not a
filesystem and the evaluator cannot traverse it by string path — authority
comes from the capability grant and opaque ids.

```text
agent-dsl-fs://{instanceId}/
  inputs/                       read-only task inputs
  runs/{runId}/
    nodes/{nodeInstanceId}/
      calls/{callId}/
        outputs/{name}
    checkpoints/{checkpointId}
```

**Blob bytes never cross the ABI.** A tool that produces an image writes the
bytes host-side and returns metadata:

```json
{
  "schemaUri": "agent-dsl://schemas/fs/artifact/1.0.0",
  "artifactId": "art_000004",
  "blobId": "sha256:8f3c...",
  "logicalPath": "runs/run_0001/nodes/nodei_000007/calls/call_000012/outputs/tile-000.png",
  "name": "tile-000.png",
  "mediaType": "image/png",
  "semanticKind": "image",
  "size": 48392,
  "sha256": "8f3c...",
  "readOnly": true,
  "labels": { "imageProfileId": "std-1280", "tileIndex": "0" }
}
```

The evaluator reasons over this metadata. It selects artifacts by semantic
kind, media type, label, or provenance without ever seeing a byte. That is what
keeps a graph cheap: a node passes a reference, not a payload.

Artifacts are create-only. A loop iteration that captures again creates a new
artifact; nothing is replaced.

## Host obligations

A host is conformant if it:

- implements every envelope above, and no other crossing;
- refuses any tool not in its registry;
- registers no final-submit capability;
- validates both directions against the JTDs;
- stores blobs itself and passes only metadata;
- writes an append-only log ordered by `sequence`;
- and preserves committed checkpoints across a restart.

`host-node` and `extension` are two implementations. The evaluator cannot tell
them apart, which is the whole point: a CLI test is evidence about the browser
runtime, not a separate thing that resembles it.
