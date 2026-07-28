// Stage 1-3 grader for ThreadBox Phase 1 katas.
//
// Invoked by promptfoo as a `javascript` assertion:
//   assert:
//     - type: javascript
//       value: file://graders/grade.mjs
//
// promptfoo calls the default export as `fn(output, context)` and
// accepts a boolean, number, or `{ pass, score, reason }`. `context`
// must carry `vars.kataId` (set per-test in promptfooconfig.yaml) so
// this grader knows which fingerprints/<kataId>.json to load.
import { execFileSync } from "node:child_process";
import { mkdirSync, writeFileSync, rmSync, readFileSync } from "node:fs";
import { randomBytes } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { checkStructure } from "./check-structure.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const EVAL_HARNESS_DIR = path.resolve(__dirname, "..");
const SDK_DIR = path.resolve(EVAL_HARNESS_DIR, "..", "sdk");
const FINGERPRINTS_DIR = path.join(EVAL_HARNESS_DIR, "fingerprints");
const ASC_BIN = path.join(SDK_DIR, "node_modules", ".bin", "asc");
// Candidate files MUST be compiled from a directory that is exactly
// ONE level below eval-harness/ -- the same nesting depth as
// eval-harness/examples/*.ts -- because system.md mandates the fixed
// relative import "../../sdk/assembly/threadbox.d" (two levels up to
// threadbox/, then into sdk/assembly/). Candidate files are therefore
// written directly inside CANDIDATE_TMP_ROOT (no extra mkdtemp
// subdirectory, which would add a third nesting level and break the
// import). Ignored via the repo's top-level .gitignore.
const CANDIDATE_TMP_ROOT = path.join(EVAL_HARNESS_DIR, ".tmp");

const MAX_SOURCE_LINES = 200;
// Kata solutions live (for grading purposes) at the same nesting depth
// as eval-harness/examples/*.ts, so the SDK's relative import path is
// always exactly two levels up. system.md mandates this exact line.
const ALLOWED_IMPORT_PATTERN = /from\s+["']\.\.\/\.\.\/sdk\/assembly\/threadbox\.d(\.ts)?["']/;

function extractCode(output) {
  // Accept any language tag (ts, typescript, assemblyscript, ...) and
  // optional CRLF so a correct solution fenced differently is not
  // scored 0 for a formatting reason.
  const fenced = /```[A-Za-z0-9_+-]*[ \t]*\r?\n([\s\S]*?)```/.exec(output);
  if (fenced) return fenced[1].trim();
  // No fenced block -- assume the whole reply is source (some models
  // omit fences even when asked for a single code block).
  return output.trim();
}

function stage1CompileCheck(source, kataId) {
  mkdirSync(CANDIDATE_TMP_ROOT, { recursive: true });
  // Written directly inside CANDIDATE_TMP_ROOT (no per-call
  // subdirectory) so the file sits exactly one level below
  // eval-harness/, matching eval-harness/examples/*.ts depth. A
  // random suffix keeps concurrent grading calls from colliding.
  const suffix = randomBytes(6).toString("hex");
  const tmpFile = path.join(CANDIDATE_TMP_ROOT, `${kataId}-${suffix}.ts`);
  writeFileSync(tmpFile, source, "utf8");
  try {
    execFileSync(ASC_BIN, [tmpFile, "--noEmit"], {
      cwd: SDK_DIR,
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { pass: true, stderr: "" };
  } catch (err) {
    const stderr = err.stderr ? err.stderr.toString() : String(err.message);
    return { pass: false, stderr };
  } finally {
    rmSync(tmpFile, { force: true });
  }
}

function stage3Lint(source) {
  const problems = [];

  const lineCount = source.split("\n").length;
  if (lineCount > MAX_SOURCE_LINES) {
    problems.push(`source has ${lineCount} lines, exceeds cap of ${MAX_SOURCE_LINES}`);
  }

  const importLines = source.match(/^import .*$/gm) || [];
  if (importLines.length === 0) {
    problems.push('missing required import from "../../sdk/assembly/threadbox.d"');
  }
  for (const line of importLines) {
    if (!ALLOWED_IMPORT_PATTERN.test(line)) {
      problems.push(`disallowed import (SDK-only imports permitted): ${line.trim()}`);
    }
  }

  // Closures/arrow functions passed directly as callback arguments to
  // .map(...) or Uni.dedupeArray(...) are a lint failure, not an asc
  // compile failure -- arrow functions type-check fine under asc, but
  // the fresh-instance-per-callback execution model requires a named
  // top-level function so the host can dispatch by Wasm table index.
  //
  // NOTE: AssemblyScript arrow params are always type-annotated, e.g.
  // `(x: string[]) => ...` -- that parenthesized param list contains
  // its own balanced "()" pair. A naive `\([^)]*=>` pattern can never
  // cross that inner ")" to reach "=>", so it would silently miss
  // every realistic closure. The alternation below explicitly allows
  // one level of parens for the arrow's own param list (or a bare
  // identifier for untyped single-param arrows) before requiring "=>".
  const closureCallbackPattern =
    /\.(map|dedupeArray)\s*(<[^>]*>)?\s*\(\s*(\([^()]*\)|[A-Za-z_$][\w$]*)\s*=>/;
  if (closureCallbackPattern.test(source)) {
    problems.push(
      "closure/arrow-function callback passed to .map or dedupeArray -- callbacks must be named top-level functions"
    );
  }

  return { pass: problems.length === 0, problems };
}

export default async function grade(output, context) {
  const kataId = context?.vars?.kataId;
  if (!kataId) {
    return {
      pass: false,
      score: 0,
      reason: "grader misconfiguration: promptfooconfig.yaml must set vars.kataId for every test",
    };
  }

  const source = extractCode(output);
  if (!source) {
    return { pass: false, score: 0, reason: "no code found in model output" };
  }

  // Stage 1: does it type-check at all? Hallucinated APIs fail here
  // with a nonzero asc exit and TS2339/TS2322 diagnostics (verified
  // empirically against this SDK during Phase 1 development).
  const compile = stage1CompileCheck(source, kataId);
  if (!compile.pass) {
    return {
      pass: false,
      score: 0,
      reason: `stage 1 (asc --noEmit) failed:\n${compile.stderr.slice(0, 2000)}`,
    };
  }

  // Stage 2: structural fingerprint -- loose required-call-set match,
  // not an exact AST match, so stylistically different but equally
  // valid solutions are not penalized.
  let fingerprint;
  try {
    const raw = readFileSync(path.join(FINGERPRINTS_DIR, `${kataId}.json`), "utf8");
    fingerprint = JSON.parse(raw);
  } catch (err) {
    return {
      pass: false,
      score: 0,
      reason: `grader misconfiguration: could not load fingerprints/${kataId}.json (${err.message})`,
    };
  }
  const structure = checkStructure(source, fingerprint);
  if (!structure.pass) {
    return {
      pass: false,
      score: 0.33,
      reason: `stage 1 passed (compiles) but stage 2 (structure) failed:\n${structure.details.join("\n")}`,
    };
  }

  // Stage 3: static lint (SDK-only imports, no closure callbacks, size cap).
  const lint = stage3Lint(source);
  if (!lint.pass) {
    return {
      pass: false,
      score: 0.66,
      reason: `stage 1+2 passed but stage 3 (lint) failed:\n${lint.problems.join("\n")}`,
    };
  }

  return { pass: true, score: 1, reason: "all 3 grading stages passed" };
}
