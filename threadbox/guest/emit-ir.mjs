/// Run a compiled authoring module once and capture the IR it emits.
///
/// This is the design-time half of the pipeline: the module's only effect is
/// the single `emit(ptr, len)` call, and what it writes is the graph.
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { dirname } from "node:path";

const [wasmPath, outPath] = process.argv.slice(2);
if (!wasmPath || !outPath) {
  process.stderr.write("usage: node emit-ir.mjs <module.wasm> <out.ir.json>\n");
  process.exit(1);
}

const bytes = await readFile(wasmPath);
let captured = null;
let instance;

({ instance } = await WebAssembly.instantiate(bytes, {
  "threadbox.ir.v1": {
    emit(ptr, len) {
      if (captured !== null) throw new Error("the guest called emit() more than once");
      captured = Buffer.from(
        new Uint8Array(instance.exports.memory.buffer, ptr, len),
      ).toString("utf8");
    },
  },
  env: {
    abort(_msg, _file, line, column) {
      throw new Error(`guest asserted at ${line}:${column}`);
    },
  },
}));

instance.exports.main();
if (captured === null) throw new Error("the guest never called emit()");

// Pretty-print so the graph is reviewable in a diff. It is a document a human
// is expected to read before a run touches anything.
const graph = JSON.parse(captured);
await mkdir(dirname(outPath), { recursive: true });
await writeFile(outPath, `${JSON.stringify(graph, null, 2)}\n`);
process.stdout.write(`${outPath} (${graph.graph.nodes.length} top-level nodes)\n`);
