// ThreadBox AssemblyScript SDK -- Phase 1 kata-authoring surface.
//
// This file is graded against with `asc --noEmit` (verified empirically:
// AssemblyScript 0.28.20 type-checks generics, private constructors,
// `@external` ambient imports, and cross-file `import { A, B } from
// "./threadbox.d"` cleanly with exit code 0; hallucinated APIs fail
// with a nonzero exit and TS2339/TS2322 diagnostics). See
// eval-harness/graders/grade.mjs stage 1.
//
// ABI NOTE: every function crossing the Wasm/host boundary below is
// declared with `@external(module, name)`. See ABI.md for the
// canonical module / function name / parameter-order table. Only
// primitives cross that boundary (i64 handles, i32 ids, UTF-16
// ptr/len string pairs) -- this is why `EndpointRef` is represented as
// a plain `string` capability name passed to `Flow.endpoint(...)`,
// never as a class instance.
//
// GENERICS CAVEAT (tracked for the Phase 2 wasmi spike): `Uni<T>`
// below type-checks under `--noEmit`, which is all Phase 1 grading
// requires. Phase 2 must additionally confirm `Uni<T>` *codegens* to
// working Wasm (not just type-checks) when compiled with
// `--exportTable --exportStart`. If real codegen fails, the fallback
// ladder is: (1) bare phantom type field, (2) nullable phantom field,
// (3) hand-monomorphized `UniText` / `UniJson` / `UniUnit` classes --
// all three share this file's wire ABI, so switching costs ~200 SDK
// lines and zero host-side changes. Kata solutions graded in Phase 1
// must only use the public static/instance methods below; the
// private `handle` field and constructor are not part of the graded
// contract and may change shape in Phase 2.

/// Opaque handle to a node in the compiled graph. Tagged and
/// generational at the host side; the Wasm side only ever holds this
/// as an opaque primitive returned by SDK calls -- never construct
/// one by hand.
export type NodeHandle = i64;

/// Opaque identifier for a registered value schema, used when
/// publishing or validating blackboard writes. Reserved for Phase 6+;
/// unused by the Phase 1 katas but kept in the ABI now so the wire
/// shape does not change later.
export type SchemaId = i32;

// ---------------------------------------------------------------------------
// Low-level host imports (see ABI.md for the canonical table). Kata
// solutions must never call these directly -- always go through
// `Flow`, `Uni`, or `ThreadBox` below. Bodies are intentionally absent
// (ambient `@external` declarations); the real host-side
// implementations do not exist yet (that is Phase 3+).
// ---------------------------------------------------------------------------

@external("threadbox.graph.v1", "node_agent")
declare function __node_agent(promptPtr: usize, promptLen: i32): NodeHandle;

@external("threadbox.graph.v1", "node_endpoint")
declare function __node_endpoint(namePtr: usize, nameLen: i32): NodeHandle;

@external("threadbox.graph.v1", "node_join_all")
declare function __node_join_all(handlesPtr: usize, handlesCount: i32): NodeHandle;

@external("threadbox.graph.v1", "node_map")
declare function __node_map(inputHandle: NodeHandle, callbackTableIndex: i32): NodeHandle;

@external("threadbox.graph.v1", "node_dedupe")
declare function __node_dedupe(inputHandle: NodeHandle, callbackTableIndex: i32): NodeHandle;

@external("threadbox.graph.v1", "node_agent_with_input")
declare function __node_agent_with_input(
  inputHandle: NodeHandle,
  promptPtr: usize,
  promptLen: i32
): NodeHandle;

@external("threadbox.trace.v1", "publish")
declare function __publish(nodeHandle: NodeHandle, schemaId: SchemaId): void;

// ---------------------------------------------------------------------------
// Public SDK surface -- this is the only API kata solutions should use.
// ---------------------------------------------------------------------------

/// A pending (not-yet-executed) unit of work in the graph, parameterized
/// by the JSON-serializable value it will eventually produce. `Uni<T>`
/// values are inert descriptions of graph structure at compile time;
/// nothing runs until the host walks the compiled graph at run time.
/// There is no way to inspect or branch on a `Uni<T>`'s value from
/// within the DSL itself -- that would require dynamic post-seal graph
/// mutation, which ThreadBox does not support (static templates only).
export class Uni<T> {
  private constructor(private readonly handle: NodeHandle) {}

  /// Wraps a raw node handle returned by a `Flow.*` or `Uni.*` call.
  /// Package-internal; kata solutions never call this directly.
  static fromHandle<T>(handle: NodeHandle): Uni<T> {
    return new Uni<T>(handle);
  }

  /// Joins a fixed, statically-known list of `Uni<T>` nodes into a
  /// single `Uni<T[]>` that resolves once every input has resolved.
  /// The list length must be a compile-time-known array literal or
  /// fixed-size collection -- ThreadBox has no dynamic post-seal graph
  /// mutation, so the join fan-in width is baked into the graph shape.
  static joinAll<T>(nodes: Uni<T>[]): Uni<T[]> {
    const handles = new Array<NodeHandle>(nodes.length);
    for (let i = 0; i < nodes.length; i++) {
      handles[i] = nodes[i].rawHandle();
    }
    return Uni.fromHandle<T[]>(__node_join_all(handles.dataStart, handles.length));
  }

  /// Applies a pure, named (never a closure or arrow function) callback
  /// to this node's eventual result. The callback must be a top-level
  /// exported function so the host can dispatch it by Wasm table index
  /// -- this is required for the fresh-instance-per-callback execution
  /// model and for deterministic replay. Passing a closure that
  /// captures outer-scope state is graded as a lint failure, not an
  /// `asc` compile failure.
  map<U>(callback: (value: T) => U): Uni<U> {
    return Uni.fromHandle<U>(__node_map(this.handle, threadbox_callback_index(callback)));
  }

  /// Removes duplicate elements from an array-valued node, keyed by a
  /// pure, named per-element callback. Only meaningful when `T` is
  /// itself an array type (for example the `string[]` produced by
  /// `Uni.joinAll`) -- this is a static method rather than an instance
  /// method on `Uni<T>` because the key callback operates on array
  /// *elements*, not on `T` as a whole.
  static dedupeArray<T, K>(input: Uni<T[]>, keyOf: (item: T) => K): Uni<T[]> {
    return Uni.fromHandle<T[]>(
      __node_dedupe(input.rawHandle(), threadbox_callback_index(keyOf))
    );
  }

  /// Spawns a new sandboxed agent node whose context includes this
  /// node's resolved value plus the given prompt. This is the only way
  /// to hand upstream data (join results, mapped values, ...) to a
  /// further agent call -- there is no other data path into an agent's
  /// context. Counts toward a kata's agent-call budget like `Flow.agent`.
  toAgent(prompt: string): Uni<string> {
    return Uni.fromHandle<string>(
      __node_agent_with_input(this.handle, changetype<usize>(prompt), prompt.length)
    );
  }

  /// Internal accessor for the raw node handle, used only by
  /// `ThreadBox.publish` and other SDK-internal plumbing. Not part of
  /// the graded kata contract.
  rawHandle(): NodeHandle {
    return this.handle;
  }
}

/// Entry points for constructing graph nodes. All node-construction
/// calls are static factory methods -- there is no way to build a node
/// except through this class.
export class Flow {
  /// Spawns a sandboxed agent node with the given prompt. Counts
  /// toward any kata's stated agent-call budget (for example, "at most
  /// three agent calls" in the parallel-review kata).
  static agent(prompt: string): Uni<string> {
    return Uni.fromHandle<string>(__node_agent(changetype<usize>(prompt), prompt.length));
  }

  /// References a capability registered in the host's endpoint
  /// registry by name (for example `"health"`, `"metrics-summary"`,
  /// `"deployment-status"`). There is no way to construct an arbitrary
  /// URL or an unregistered endpoint from within the DSL -- referencing
  /// an unregistered name fails on the host side when the graph is
  /// compiled, not at Wasm runtime, and is not something a kata
  /// solution can work around.
  static endpoint(name: string): Uni<string> {
    return Uni.fromHandle<string>(__node_endpoint(changetype<usize>(name), name.length));
  }
}

/// Marks the terminal output of a graph. Exactly one `ThreadBox.publish`
/// call is required per program -- this is graded structurally in
/// Phase 1 (see `check-structure.mjs`).
export class ThreadBox {
  static publish<T>(result: Uni<T>): void {
    __publish(result.rawHandle(), 0);
  }
}

// ---------------------------------------------------------------------------
// Table-index resolution placeholder. Real table-index dispatch is one
// of the six sub-proofs required in the Phase 2 wasmi spike; this
// Phase 1 stub only needs to type-check so `Uni.map` / `Uni.dedupe`
// have a body to call. Kata solutions never call this directly.
// ---------------------------------------------------------------------------
function threadbox_callback_index<F>(callback: F): i32 {
  return 0;
}
