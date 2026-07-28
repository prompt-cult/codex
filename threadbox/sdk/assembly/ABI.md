# ThreadBox Wasm ABI (Phase 1 snapshot)

This is the canonical reference for every function that crosses the
Wasm/host boundary in the ThreadBox AssemblyScript SDK
(`assembly/threadbox.d.ts`). It exists so the Phase 5 sandboxed
compiler's import-namespace allowlist has one source of truth, and so
the Phase 2 wasmi spike has a fixed target to prove against.

**Status: Phase 1 snapshot, not yet implemented or spiked.** Nothing in
this table has been proven to codegen correctly under wasmi (only
type-checked under `asc --noEmit`, which does not touch table export,
i64 marshaling, or string ptr/len passing at all). Treat every row as
a proposal the Phase 2 spike must confirm or revise.

## Primitives-only rule

Only these Wasm-native value types may cross the host/guest boundary:

| Wasm type | ThreadBox meaning |
|---|---|
| `i64` | `NodeHandle` -- tagged, generational, opaque to the guest |
| `i32` | `SchemaId`, table indices, byte lengths, small counts |
| `usize` (i32 on wasm32) | UTF-16 string data pointer, paired with an `i32` length |

Nothing else crosses the boundary. In particular:

- `EndpointRef` is a plain UTF-16 string capability **name**
  (`"health"`, `"metrics-summary"`, ...), never a class instance --
  classes cannot be marshaled across the Wasm ABI.
- Callbacks passed to `Uni.map` / `Uni.dedupe` cross the boundary as an
  `i32` Wasm table index, resolved by `threadbox_callback_index`
  (currently a Phase 1 stub returning `0`; real table-index resolution
  is a Phase 2 spike sub-proof).
- Arrays (for example the `nodes` parameter to `Uni.joinAll`) cross as
  an `(dataStart: usize, length: i32)` pair, matching AssemblyScript's
  own `Array<T>.dataStart` layout.

## Import table

Every import is namespaced `threadbox.<subsystem>.v1` so the Phase 5
sandbox compiler can allowlist by module prefix and version together
(a `v2` bump is a breaking ABI change, not a silent behavior change).

| Module | Function | Params (in order) | Returns | Notes |
|---|---|---|---|---|
| `threadbox.graph.v1` | `node_agent` | `promptPtr: usize, promptLen: i32` | `NodeHandle` | Spawns a sandboxed agent node. Counts toward a kata's agent-call budget. |
| `threadbox.graph.v1` | `node_endpoint` | `namePtr: usize, nameLen: i32` | `NodeHandle` | References a registered capability by name. Host rejects unregistered names at graph-compile time. |
| `threadbox.graph.v1` | `node_join_all` | `handlesPtr: usize, handlesCount: i32` | `NodeHandle` | Fan-in join over a statically-sized handle array. |
| `threadbox.graph.v1` | `node_map` | `inputHandle: NodeHandle, callbackTableIndex: i32` | `NodeHandle` | Pure named-callback transform. |
| `threadbox.graph.v1` | `node_dedupe` | `inputHandle: NodeHandle, callbackTableIndex: i32` | `NodeHandle` | Pure named-key dedupe over an array-valued node's elements. |
| `threadbox.graph.v1` | `node_agent_with_input` | `inputHandle: NodeHandle, promptPtr: usize, promptLen: i32` | `NodeHandle` | Spawns a further agent node whose context includes an upstream node's resolved value. The only data path from pure computation back into an agent call. |
| `threadbox.trace.v1` | `publish` | `nodeHandle: NodeHandle, schemaId: SchemaId` | `void` | Marks the terminal output of the graph. Exactly one call per program. |
| `threadbox.value.v1` | *(reserved)* | -- | -- | Reserved for Phase 6 callback opcode ABI (`value_*` family per the ThreadBox synthesis docs). Not used by any Phase 1 kata. |

## Six sub-proofs required before this table is trusted (Phase 2 spike)

1. `asc --exportTable --exportStart` output actually loads in `wasmi`.
2. An `i64` value round-trips correctly across a host import call.
3. Table-index dispatch (guest calls a callback by `i32` index, host
   invokes the correct guest function) works end-to-end.
4. UTF-16 string reads from guest linear memory via `(ptr, len)` work
   for realistic prompt-length strings.
5. `Uni<T>` compiles to working Wasm codegen, not just `--noEmit`
   type-checks (see the generics caveat in `threadbox.d.ts`). If it
   fails, apply the 3-step fallback ladder documented there.
6. A trap inside one fresh-Wasmi-instance callback does not poison
   sibling callbacks or the host process.

## Change policy

Any change to a row in the import table above is a breaking ABI
change and requires bumping the module version suffix (`v1` -> `v2`)
for that subsystem, not an in-place edit. This keeps the Phase 5
sandbox compiler's allowlist auditable: an old compiled kata solution
either matches a known ABI version exactly, or fails the allowlist
check loudly instead of silently linking against a changed host
function.
