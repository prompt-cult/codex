/// The serialization walker. Reads the arena `ir.ts` built and writes the
/// `threadbox.ir.v1` envelope documented in `IR.md`, then calls the one
/// import that crosses the guest/host boundary. Nothing here mutates the
/// arena; nothing here runs more than once.

import { arena, Node, Kind } from "./ir";
import { ModelSpec } from "./models";

/// The one import. The host reads `len` bytes of UTF-8 from guest linear
/// memory starting at `ptr`. See `README.md`'s ABI table — this row is an
/// ABI and changing it takes a version bump in the module name, never an
/// in-place edit.
// @ts-ignore: decorator
@external("threadbox.ir.v1", "emit")
declare function emit(ptr: usize, len: i32): void;

/// Escape one JSON string per RFC 8259: quote, backslash, and control
/// characters get escaped; everything else passes through unchanged.
function jsonString(value: string): string {
  let out = "\"";
  for (let i = 0; i < value.length; i++) {
    const c = value.charCodeAt(i);
    if (c == 0x22) {
      out += "\\\"";
    } else if (c == 0x5c) {
      out += "\\\\";
    } else if (c == 0x0a) {
      out += "\\n";
    } else if (c == 0x0d) {
      out += "\\r";
    } else if (c == 0x09) {
      out += "\\t";
    } else if (c < 0x20) {
      const hex = c.toString(16);
      out += hex.length < 2 ? "\\u000" + hex : "\\u00" + hex;
    } else {
      out += String.fromCharCode(c);
    }
  }
  out += "\"";
  return out;
}

/// Serialize a `ModelSpec` in the slot order `IR.md` fixes for golden byte
/// comparisons: `role`, `tier`, `vendor`, `model`, `think`, `contextWindow`,
/// `driver`. Absent slots are omitted entirely rather than emitted as null.
function jsonModel(model: ModelSpec): string {
  let out = "{";
  let first = true;
  if (model.role !== null) {
    out += "\"role\":" + jsonString(model.role!);
    first = false;
  }
  if (model.tier !== null) {
    out += (first ? "" : ",") + "\"tier\":" + jsonString(model.tier!);
    first = false;
  }
  if (model.vendor !== null) {
    out += (first ? "" : ",") + "\"vendor\":" + jsonString(model.vendor!);
    first = false;
  }
  if (model.model !== null) {
    out += (first ? "" : ",") + "\"model\":" + jsonString(model.model!);
    first = false;
  }
  if (model.thinkLevel !== null) {
    out += (first ? "" : ",") + "\"think\":" + jsonString(model.thinkLevel!);
    first = false;
  }
  if (model.contextWindow != 0) {
    out += (first ? "" : ",") + "\"contextWindow\":" + model.contextWindow.toString();
    first = false;
  }
  if (model.driver !== null) {
    out += (first ? "" : ",") + "\"driver\":" + jsonString(model.driver!);
    first = false;
  }
  out += "}";
  return out;
}

/// Serialize `parents` as a JSON array of arena indices, in order.
function jsonParents(parents: Array<i32>): string {
  let out = "[";
  for (let i = 0; i < parents.length; i++) {
    if (i > 0) out += ",";
    out += parents[i].toString();
  }
  out += "]";
  return out;
}

/// Serialize one node's kind-specific fields, in the order `IR.md`'s table
/// fixes for that kind, each prefixed with its own leading comma. Common
/// fields (`i`, `kind`, `parents`) are the caller's job.
function jsonNodeFields(node: Node): string {
  const kind = node.kind;
  if (kind == Kind.LoadJson) {
    return ",\"name\":" + jsonString(node.name!);
  } else if (kind == Kind.Screenshot) {
    return "";
  } else if (kind == Kind.Scale) {
    return ",\"width\":" + node.width.toString();
  } else if (kind == Kind.Locate) {
    let out = ",\"description\":" + jsonString(node.description!);
    if (node.model !== null) out += ",\"model\":" + jsonModel(node.model!);
    return out;
  } else if (kind == Kind.Click) {
    return "";
  } else if (kind == Kind.Type) {
    return ",\"valueKind\":" + jsonString(node.valueKind!) +
           ",\"value\":" + jsonString(node.value!);
  } else if (kind == Kind.Verify) {
    let out = ",\"assertion\":" + jsonString(node.assertion!);
    if (node.model !== null) out += ",\"model\":" + jsonModel(node.model!);
    return out;
  } else if (kind == Kind.Retry) {
    return ",\"bound\":" + node.bound.toString();
  } else if (kind == Kind.Fallback) {
    return "";
  } else if (kind == Kind.Branch) {
    return "";
  } else if (kind == Kind.ForEach) {
    return ",\"over\":" + jsonString(node.over!) +
           ",\"body\":" + node.body.toString();
  } else if (kind == Kind.Publish) {
    return "";
  }
  // Unreachable: `push()` in `ir.ts` only ever constructs one of the
  // twelve kinds named in `Kind`.
  assert(false, "unknown node kind: " + kind);
  return "";
}

/// Serialize one node in full: common fields first, kind-specific fields
/// after, exactly as `IR.md` fixes the order.
function jsonNode(node: Node): string {
  return "{\"i\":" + node.i.toString() +
         ",\"kind\":" + jsonString(node.kind) +
         ",\"parents\":" + jsonParents(node.parents) +
         jsonNodeFields(node) +
         "}";
}

/// Walk the arena, assert exactly one `Publish` exists, serialize the
/// `threadbox.ir.v1` envelope, and call the single import. This is the
/// last statement of `main` and the only effect the module has.
export function emitGraph(): void {
  let publishCount = 0;
  for (let i = 0; i < arena.length; i++) {
    if (arena[i].kind == Kind.Publish) publishCount++;
  }
  assert(
    publishCount == 1,
    "graph has " + publishCount.toString() + " Publish nodes; exactly one is required"
  );

  let nodesJson = "";
  for (let i = 0; i < arena.length; i++) {
    if (i > 0) nodesJson += ",";
    nodesJson += jsonNode(arena[i]);
  }
  const json = "{\"ir\":\"threadbox.ir.v1\",\"nodes\":[" + nodesJson + "]}";

  const bytes = String.UTF8.encode(json, false);
  emit(changetype<usize>(bytes), bytes.byteLength);
}
