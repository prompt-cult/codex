/// The graph builder. Every combinator appends one node to the guest's
/// own arena and returns a `Step` (or `Doc`, or `FieldRef`) referencing
/// it by index. Nothing here executes; building the arena is the whole
/// effect of running `main`. See `IR.md` for the serialized form this
/// arena is written into, and `DSL.md` for the full construct list.

import { ModelSpec } from "./models";

/// The twelve node kinds. A kind not named here does not exist.
export namespace Kind {
  export const LoadJson: string = "LoadJson";
  export const Screenshot: string = "Screenshot";
  export const Scale: string = "Scale";
  export const Locate: string = "Locate";
  export const Click: string = "Click";
  export const Type: string = "Type";
  export const Verify: string = "Verify";
  export const Retry: string = "Retry";
  export const Fallback: string = "Fallback";
  export const Branch: string = "Branch";
  export const ForEach: string = "ForEach";
  export const Publish: string = "Publish";
}

/// One arena slot. Most fields apply to exactly one kind; `emit.ts`
/// reads only the fields that kind declares in `IR.md`. This flat shape
/// is the mechanical guest-side counterpart of the sealed-variant graph
/// `threadbox-ir` reconstructs on the Rust side — the arena is a plain
/// growable array, not a tagged union, because AssemblyScript has no
/// sealed types to model one.
export class Node {
  i: i32;
  kind: string;
  parents: Array<i32>;

  // LoadJson
  name: string | null = null;

  // Scale
  width: i32 = 0;

  // Locate, Verify
  description: string | null = null; // Locate
  assertion: string | null = null; // Verify
  model: ModelSpec | null = null; // Locate, Verify

  // Type
  valueKind: string | null = null; // "literal" | "field"
  value: string | null = null;

  // Retry
  bound: i32 = 0;

  // ForEach
  over: string | null = null;
  body: i32 = -1;

  constructor(i: i32, kind: string, parents: Array<i32>) {
    this.i = i;
    this.kind = kind;
    this.parents = parents;
  }
}

/// The guest's own arena. Module-level, single-shot: `main()` runs
/// once, builds this array, and hands it to `emitGraph()`. There is no
/// reentry and no reset between runs because there is only one run.
export const arena: Array<Node> = new Array<Node>();

/// Append one node and return its index. The only place a `Node` is
/// constructed.
function push(kind: string, parents: Array<i32>): i32 {
  const index = arena.length;
  arena.push(new Node(index, kind, parents));
  return index;
}

/// A cursor into the arena. `index` is the tip of the chain built so
/// far; `rootIndex` is the first node built in *this* chain — the node
/// a `.then()` from an earlier chain, or an `.after()` onto a later
/// one, attaches to. Every combinator below propagates `rootIndex`
/// unchanged from `this`; only a chain-starting construct
/// (`screenshot()`, `Multi.over(...).forEach(...)`, `.orElse(...)`,
/// `.branch(...)`, `.publish()`) sets `rootIndex` to its own index.
export class Step {
  index: i32;
  rootIndex: i32;

  constructor(index: i32, rootIndex: i32) {
    this.index = index;
    this.rootIndex = rootIndex;
  }

  /// Normalize the current view to `width` pixels with padding.
  scale(width: i32): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    const i = push(Kind.Scale, parents);
    arena[i].width = width;
    return new Step(i, this.rootIndex);
  }

  /// Resolve `description` to viewport coordinates against the current
  /// view, optionally naming a vision model explicitly.
  locate(description: string, model: ModelSpec | null = null): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    const i = push(Kind.Locate, parents);
    arena[i].description = description;
    arena[i].model = model;
    return new Step(i, this.rootIndex);
  }

  /// Act at the coordinates resolved by the current step.
  click(): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    const i = push(Kind.Click, parents);
    return new Step(i, this.rootIndex);
  }

  /// Enter a literal value at the resolved coordinates.
  type(value: string): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    const i = push(Kind.Type, parents);
    arena[i].valueKind = "literal";
    arena[i].value = value;
    return new Step(i, this.rootIndex);
  }

  /// Enter a value read from an input document at the resolved
  /// coordinates. `ref` also becomes a parent, so the `Type` node
  /// depends on both the click and the document it reads.
  typeField(ref: FieldRef): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    parents.push(ref.docIndex);
    const i = push(Kind.Type, parents);
    arena[i].valueKind = "field";
    arena[i].value = ref.path;
    return new Step(i, this.rootIndex);
  }

  /// Second-opinion check against the current view, yielding a boolean
  /// that `.branch()` or `.attempts()` can consume.
  confirm(assertion: string, model: ModelSpec | null = null): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    const i = push(Kind.Verify, parents);
    arena[i].assertion = assertion;
    arena[i].model = model;
    return new Step(i, this.rootIndex);
  }

  /// Bounded repetition of the chain ending at `this`. `bound` must be
  /// a positive integer literal.
  attempts(bound: i32): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    const i = push(Kind.Retry, parents);
    arena[i].bound = bound;
    return new Step(i, this.rootIndex);
  }

  /// The escalation ladder: use `alternative`'s chain when this chain
  /// does not succeed. Starts a fresh chain — the `Fallback` node is
  /// its own root.
  orElse(alternative: Step): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    parents.push(alternative.index);
    const i = push(Kind.Fallback, parents);
    return new Step(i, i);
  }

  /// The only conditional. Consumes a `Verify` (`this`). `whenTrue` and
  /// `whenFalse` are named top-level functions, each building its own
  /// self-contained chain from the point passed in. Starts a fresh
  /// chain.
  branch(
    whenTrue: (from: Step) => Step,
    whenFalse: (from: Step) => Step
  ): Step {
    const trueTail = whenTrue(this);
    const falseTail = whenFalse(this);
    const parents = new Array<i32>();
    parents.push(this.index);
    parents.push(trueTail.index);
    parents.push(falseTail.index);
    const i = push(Kind.Branch, parents);
    return new Step(i, i);
  }

  /// Sequencing edge: attach `this` as a trailing parent of the root of
  /// `next`'s chain — the first node built in that chain, not its tail
  /// — after that node's own parents. Returns `next` unchanged.
  then(next: Step): Step {
    arena[next.rootIndex].parents.push(this.index);
    return next;
  }

  /// Sequencing edge, the other direction: attach `prior` as a trailing
  /// parent of the root of *this* chain. Returns `this` unchanged.
  /// `a.then(b)` and `b.after(a)` perform the identical mutation.
  after(prior: Step): Step {
    arena[this.rootIndex].parents.push(prior.index);
    return this;
  }

  /// The terminal node. Exactly one per program; `emitGraph()` asserts
  /// this before serializing. Starts a fresh chain, though nothing
  /// chains after it.
  publish(): Step {
    const parents = new Array<i32>();
    parents.push(this.index);
    const i = push(Kind.Publish, parents);
    return new Step(i, i);
  }
}

/// An opaque reference to a value inside a document loaded by
/// `loadJson`. Produced by `Doc.field()` (a scalar) or `Doc.fields()`
/// (an array, the only thing `Multi.over()` accepts) — the two are
/// structurally identical; the distinction is the caller's, not the
/// type's.
export class FieldRef {
  docIndex: i32;
  path: string;

  constructor(docIndex: i32, path: string) {
    this.docIndex = docIndex;
    this.path = path;
  }
}

/// A loaded input document. Declares the guest's only form of input:
/// the host provides no other way to read data into the graph.
export class Doc {
  index: i32;

  constructor(index: i32) {
    this.index = index;
  }

  /// A scalar field of this document.
  field(path: string): FieldRef {
    return new FieldRef(this.index, path);
  }

  /// An array field of this document.
  fields(path: string): FieldRef {
    return new FieldRef(this.index, path);
  }
}

/// Declare a named input document. `name` identifies which document to
/// supply at run time; the guest never reads a filesystem or network to
/// find it.
export function loadJson(name: string): Doc {
  const parents = new Array<i32>();
  const i = push(Kind.LoadJson, parents);
  arena[i].name = name;
  return new Doc(i);
}

/// Capture the current view. Starts a fresh chain — a `Screenshot` node
/// is always its own chain root.
export function screenshot(): Step {
  const parents = new Array<i32>();
  const i = push(Kind.Screenshot, parents);
  return new Step(i, i);
}

/// The array-iteration entry point: `Multi.over(ref).forEach(body)`.
export class Multi {
  static over(ref: FieldRef): ForEachBuilder {
    return new ForEachBuilder(ref);
  }
}

/// Bound to one array field by `Multi.over()`; `.forEach()` finishes
/// building the `ForEach` node.
export class ForEachBuilder {
  ref: FieldRef;

  constructor(ref: FieldRef) {
    this.ref = ref;
  }

  /// Iterate the body subgraph once per element of the array named by
  /// `Multi.over()`'s argument. `body` is a named top-level function;
  /// it receives a `FieldRef` naming one element (path `"<over>[]"`)
  /// and must return the tail of a self-contained chain. Starts a
  /// fresh chain — the `ForEach` node is its own root.
  forEach(body: (element: FieldRef) => Step): Step {
    const elementRef = new FieldRef(this.ref.docIndex, this.ref.path + "[]");
    const bodyTail = body(elementRef);
    const parents = new Array<i32>();
    parents.push(this.ref.docIndex);
    const i = push(Kind.ForEach, parents);
    arena[i].over = this.ref.path;
    arena[i].body = bodyTail.index;
    return new Step(i, i);
  }
}
