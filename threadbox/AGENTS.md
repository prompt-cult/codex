# AGENTS.md — ThreadBox

Conventions and verification for anyone, human or model, editing anything under
`threadbox/`. Read `README.md` for what the system is, `ABI.md` for the
boundary, `IR.md` for the graph, `DSL.md` for the authoring language.

## Verification commands

| Scope | Command | Run from |
|---|---|---|
| Schemas and generated validators | `npm run codegen` | `threadbox` |
| Guest type-check | `npm run check` | `threadbox` |
| Evaluator, native | `cargo test` | `threadbox/evaluator` |
| Everything | `npm test` | `threadbox` |
| End to end against the twin | `npm run e2e` | `threadbox` |

Do not filter this output. If it is too large, narrow the target — one crate
with `-p`, one test file — rather than piping through `head` or a pattern
match. Filtering hides the unexpected, which is the only thing worth running a
test for.

## Non-negotiable constraints

- **Documentation before code.** `IR.md` and `ABI.md` are specifications, not
  descriptions. A node kind not in `IR.md` does not exist. An envelope field
  not in `ABI.md` does not exist.
- **The evaluator is pure.** No clock, no randomness, no filesystem, no
  network, no browser API. If it needs something, it asks the host.
- **Blobs never cross the ABI.** Tools write bytes host-side and return
  metadata. A node passes a reference, not a payload.
- **Identity is deterministic.** Two runs of the same graph over the same
  inputs produce byte-identical logs. Reproducibility beats global uniqueness.
- **No final-submit capability** in any registry, in any host, at any
  milestone. Absence is the control, not a prompt instruction.
- **Generated code is never committed.** `jtd-codegen` output is a build
  artifact. Hand-editing it is a defect.
- **Production execution is the browser WebAssembly runtime.** Node builds and
  tests artifacts; it must not become a second production runtime. The CLI host
  exists so tests exercise the real evaluator, not to ship.
- **The extension loads only packaged local code.** No remote scripts.
- Keep credentials, provider URLs, and customer data out of source and
  committed fixtures.

## Provider allowlist and data residency

Model providers are allowlisted by data-residency policy, enforced as
configuration before dispatch.

- A provider whose serving path is not on the allowlist is **prohibited by
  default**. A run that names one fails before any network call.
- **OpenCode Go is prohibited** for this work on data-residency grounds.
- The prohibition is not overridable by a graph, a prompt, or a model. It is a
  host configuration gate.
- The only exception is a payload the operator has personally reviewed and
  sanitised, as a deliberate act.

In this milestone the only registered binding is `fixture`, which is offline.
Live model calls are never part of the default test gate.

## Rejection tests

Reject a change if any of these is true:

- The graph logic hardcodes capture → model → click.
- Capture and image scaling are fused because one model happens to accept both.
- A provider adapter or a leaf tool chooses the next node.
- A raw, unnormalised capture reaches a model-bound edge.
- A node receives every prior artifact because selecting was inconvenient.
- A model is asked to perform a filter the query node can express.
- Success is a text answer rather than a checked postcondition.
- A model verdict overrides a failed observation or a failed guard.
- A loop iteration overwrites an earlier artifact.
- The DSL can reach host files, browser APIs, secrets, or unregistered modules.
- Swapping one compatible vision binding for another requires editing the graph.
- Modality compatibility exists only in prompt prose.
- A final-submit tool exists with instructions not to use it.

## Evidence discipline

- A model CLI smoke does not prove a browser extension.
- A screenshot does not prove a persisted value.
- A unit test does not prove service-worker suspension and recovery.
- A provider alias does not prove which model answered.
- A valid JTD does not prove business eligibility.
- A successful model response does not authorise a page action.
- Chrome proof does not imply Edge proof.

The twin's action log is the oracle. A transcript is not evidence, and neither
is a model's claim to have finished.

## Unix composition

- Build stages communicate through files, stdout/stderr, and exit status.
- Each script does one job and is independently testable.
- AssemblyScript compilation, wasm packaging, and extension packaging stay
  separate commands.
- Tests instantiate the same wasm bytes that are packaged into the extension.

## Rust style

- Data first: plain structs and enums, exhaustive `match`, no trait
  hierarchies, no wildcard arms over closed sets.
- Inline `format!` arguments: `format!("{count} nodes")`.
- Modules stay under 500 lines excluding tests. Add a module rather than
  growing one.
- Public entry points validate inputs and return descriptive errors naming the
  constraint, the actual value, and the expectation. `assert!` is for internal
  invariants only.
- The wasm `extern` layer is a thin shell over a pure inner API, so every path
  is reachable from a native test.

## Guest style

- AssemblyScript subset only: no overloading, no closures, no object literals.
- Loop bodies are named top-level functions.
- Every loop bound is an integer literal.
- Node ids are stable kebab-case strings; renaming one is a version change.
- Documentation comments use `///`.

## Error message standard

Name the constraint, the offending value, and the expectation.

Good:

```text
Loop at index 5 (each-field) has max 0; max must be a positive integer literal
tool "form.submit" is not in the registry; register a tool before a graph may name it
provider "opencode-go" is not on the data-residency allowlist; refusing to dispatch
```

Bad: `invalid graph`, `bad value`, `validation failed`.

## Logging

- Diagnostics to standard error, results to standard output. A tool that
  pollutes standard output is unusable in a pipe.
- The host log is append-only and ordered by `sequence`. Never truncate it to
  make output prettier; every step is evidence.
- Redact secrets and provider error bodies. Never log an authorization header.

## Commits

- Start with a short imperative description. No `feat:`/`fix:` prefixes.
- Say what was achieved and how to verify it: the command and the expected
  result.
- No promotional footer, no `Co-Authored-By`.
- Keep commits atomic. Never commit a failing test, dead code, or a disabled
  feature.
