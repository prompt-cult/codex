/// The authoring DSL.
///
/// A library embedded in AssemblyScript whose only purpose is to build an IR
/// graph in the guest's own linear memory and serialize it once. Nothing here
/// performs I/O; there is no host round-trip during construction, so no
/// combinator can inspect a live value.
///
/// See `DSL.md` for the vocabulary and `IR.md` for the document this emits.

/// The one import. The host reads `len` bytes of UTF-8 from guest memory at
/// `ptr`. Changing this row is a breaking change and takes a version bump in
/// the module name, never an in-place edit.
// @ts-ignore: decorator
@external("threadbox.ir.v1", "emit")
declare function emit(ptr: usize, len: i32): void;

/// A handle to a node in the arena currently being built. It carries an index
/// and nothing else: there is no way to read a value back out, because there
/// is no value yet.
export class Node {
  index: i32;
  constructor(index: i32) {
    this.index = index;
  }
}

/// One arena. The top-level graph is a frame; so is each loop body and each
/// branch arm, which is what keeps nested indices local and every edge
/// pointing backward.
class Frame {
  nodes: Array<string>;
  constructor() {
    this.nodes = new Array<string>();
  }
}

const frames: Array<Frame> = new Array<Frame>();
let graphId: string = "unnamed";
let graphVersion: string = "0.1.0";

function current(): Frame {
  if (frames.length == 0) frames.push(new Frame());
  return frames[frames.length - 1];
}

/// Escape a string for JSON, per RFC 8259.
function q(value: string): string {
  let out = "\"";
  for (let i = 0; i < value.length; i++) {
    const c = value.charCodeAt(i);
    if (c == 0x22) out += "\\\"";
    else if (c == 0x5c) out += "\\\\";
    else if (c == 0x0a) out += "\\n";
    else if (c == 0x0d) out += "\\r";
    else if (c == 0x09) out += "\\t";
    else if (c < 0x20) {
      const hex = c.toString(16);
      out += hex.length < 2 ? "\\u000" + hex : "\\u00" + hex;
    } else out += String.fromCharCode(c);
  }
  return out + "\"";
}

function joinNodes(nodes: Array<string>): string {
  let out = "";
  for (let i = 0; i < nodes.length; i++) {
    if (i > 0) out += ",";
    out += nodes[i];
  }
  return out;
}

function parentsJson(parents: Array<Node>): string {
  let out = "[";
  for (let i = 0; i < parents.length; i++) {
    if (i > 0) out += ",";
    out += parents[i].index.toString();
  }
  return out + "]";
}

/// Append a node to the current frame. `head` carries the kind-specific
/// fields, already serialized.
function push(id: string, kind: string, parents: Array<Node>, extra: string): Node {
  const frame = current();
  const index = frame.nodes.length;
  let json = "{\"i\":" + index.toString() +
             ",\"id\":" + q(id) +
             ",\"kind\":" + q(kind) +
             ",\"parents\":" + parentsJson(parents);
  if (extra.length > 0) json += extra;
  json += "}";
  frame.nodes.push(json);
  return new Node(index);
}

// ---------------------------------------------------------------- parents

export function p0(): Array<Node> {
  return new Array<Node>();
}

export function p1(a: Node): Array<Node> {
  const out = new Array<Node>();
  out.push(a);
  return out;
}

export function p2(a: Node, b: Node): Array<Node> {
  const out = new Array<Node>();
  out.push(a);
  out.push(b);
  return out;
}

export function p3(a: Node, b: Node, c: Node): Array<Node> {
  const out = new Array<Node>();
  out.push(a);
  out.push(b);
  out.push(c);
  return out;
}

// ---------------------------------------------------------------- graph

/// Name the graph. Called once, before any node.
export function graph(id: string, version: string): void {
  graphId = id;
  graphVersion = version;
  frames.push(new Frame());
}

// ---------------------------------------------------------------- nodes

export function input(id: string, name: string, schemaUri: string): Node {
  return push(id, "Input", p0(),
    ",\"name\":" + q(name) + ",\"schemaUri\":" + q(schemaUri));
}

export function tool(id: string, name: string, inputExpr: string, parents: Array<Node>): Node {
  return push(id, "Tool", parents,
    ",\"tool\":" + q(name) + ",\"input\":" + q(inputExpr));
}

export function agent(
  id: string,
  binding: string,
  prompt: string,
  inputExpr: string,
  outputSchemaUri: string,
  parents: Array<Node>
): Node {
  return push(id, "Agent", parents,
    ",\"binding\":" + q(binding) +
    ",\"prompt\":" + q(prompt) +
    ",\"input\":" + q(inputExpr) +
    ",\"outputSchemaUri\":" + q(outputSchemaUri));
}

export function transform(id: string, expr: string, parents: Array<Node>): Node {
  return push(id, "Transform", parents, ",\"expr\":" + q(expr));
}

/// Require a condition. The value passes through unchanged: a guard is a gate,
/// not a projection.
export function assertThat(id: string, expr: string, parent: Node): Node {
  return push(id, "Guard", p1(parent), ",\"mode\":\"assert\",\"expr\":" + q(expr));
}

/// Require the value to satisfy a JTD.
export function validateWith(id: string, schemaUri: string, parent: Node): Node {
  return push(id, "Guard", p1(parent), ",\"mode\":\"validate\",\"schemaUri\":" + q(schemaUri));
}

export function human(id: string, classification: string, inputExpr: string, parent: Node): Node {
  return push(id, "Human", p1(parent),
    ",\"classification\":" + q(classification) + ",\"input\":" + q(inputExpr));
}

export function checkpoint(id: string, label: string, inputExpr: string, parent: Node): Node {
  return push(id, "Checkpoint", p1(parent),
    ",\"label\":" + q(label) + ",\"input\":" + q(inputExpr));
}

export function terminal(id: string, status: string, outcome: string, parent: Node): Node {
  return push(id, "Terminal", p1(parent),
    ",\"status\":" + q(status) + ",\"outcome\":" + q(outcome));
}

/// Bounded iteration. `body` is a named top-level function that builds the
/// nested arena; it is called once, at construction time, and the arena it
/// builds is reused for every iteration at run time.
export function loop(
  id: string,
  varName: string,
  overExpr: string,
  max: i32,
  parent: Node,
  body: () => Node
): Node {
  frames.push(new Frame());
  body();
  const inner = frames.pop();
  const bodyJson = ",\"body\":{\"nodes\":[" + joinNodes(inner.nodes) + "]}";
  return push(id, "Loop", p1(parent),
    ",\"var\":" + q(varName) +
    ",\"over\":" + q(overExpr) +
    ",\"max\":" + max.toString() +
    bodyJson);
}

/// The only conditional. Both arms are named top-level functions building
/// their own nested arenas; only the taken arm runs.
export function branch(
  id: string,
  condExpr: string,
  parent: Node,
  whenTrue: () => Node,
  whenFalse: () => Node
): Node {
  frames.push(new Frame());
  whenTrue();
  const t = frames.pop();

  frames.push(new Frame());
  whenFalse();
  const f = frames.pop();

  return push(id, "Branch", p1(parent),
    ",\"cond\":" + q(condExpr) +
    ",\"whenTrue\":{\"nodes\":[" + joinNodes(t.nodes) + "]}" +
    ",\"whenFalse\":{\"nodes\":[" + joinNodes(f.nodes) + "]}");
}

// ---------------------------------------------------------------- emit

/// Serialize the graph and call the single import. The last statement of
/// `main`, and the only effect the module has.
export function emitGraph(): void {
  assert(frames.length == 1, "emitGraph() called with an unclosed loop body or branch arm");
  const top = frames[0];

  let terminals = 0;
  for (let i = 0; i < top.nodes.length; i++) {
    if (top.nodes[i].includes("\"kind\":\"Terminal\"")) terminals++;
  }
  assert(terminals == 1, "a graph must have exactly one Terminal");

  const json = "{\"ir\":\"threadbox.ir.v2\",\"graph\":{" +
    "\"id\":" + q(graphId) +
    ",\"version\":" + q(graphVersion) +
    ",\"nodes\":[" + joinNodes(top.nodes) + "]}}";

  const bytes = String.UTF8.encode(json, false);
  emit(changetype<usize>(bytes), bytes.byteLength);
}
