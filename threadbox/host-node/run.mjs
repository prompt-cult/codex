/// Run a graph against a live twin.
///
/// Usage:
///   node host-node/run.mjs --ir <graph.ir.json> [--base-url URL]
///                          [--submission FILE] [--corpus FILE]
///                          [--provider fixture] [--strict|--record]
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { run } from "./host.mjs";
import { FixtureProvider } from "./provider.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function parseArgs(argv) {
  const args = {
    ir: resolve(root, "guest/build/acme-motor-quotes.ir.json"),
    baseUrl: "http://127.0.0.1:3456",
    submission: resolve(root, "harness/twin/fixtures/submission.json"),
    corpus: resolve(root, "host-node/fixtures/model-corpus.json"),
    wasm: resolve(root, "evaluator/target/wasm32-unknown-unknown/release/threadbox_evaluator.wasm"),
    mode: "strict",
  };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    switch (flag) {
      case "--ir": args.ir = resolve(value); i += 1; break;
      case "--base-url": args.baseUrl = value; i += 1; break;
      case "--submission": args.submission = resolve(value); i += 1; break;
      case "--corpus": args.corpus = resolve(value); i += 1; break;
      case "--wasm": args.wasm = resolve(value); i += 1; break;
      case "--provider": i += 1; break; // only `fixture` exists in this milestone
      case "--strict": args.mode = "strict"; break;
      case "--record": args.mode = "record"; break;
      default:
        throw new Error(`unrecognized argument "${flag}"`);
    }
  }
  return args;
}

const args = parseArgs(process.argv.slice(2));

const ir = await readFile(args.ir, "utf8");
const submission = JSON.parse(await readFile(args.submission, "utf8"));
const provider = await FixtureProvider.load(args.corpus, args.mode);

const outcome = await run({
  wasmPath: args.wasm,
  ir,
  inputs: { submission },
  baseUrl: args.baseUrl,
  provider,
});

// The graph goes to standard output; diagnostics go to standard error, so the
// result is usable in a pipe.
process.stderr.write(
  `${outcome.finished.status}: ${outcome.finished.outcome}\n` +
    `  tool calls: ${outcome.requests.length}\n` +
    `  artifacts:  ${outcome.artifacts.count}\n` +
    `  checkpoints: ${outcome.log.checkpoints.length}\n`,
);
process.stdout.write(`${JSON.stringify(outcome.finished, null, 2)}\n`);
process.exit(outcome.finished.status === "success" ? 0 : 1);
