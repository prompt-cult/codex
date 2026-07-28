# ThreadBox

ThreadBox runs a typed agent DSL as WebAssembly inside a browser extension, to
drive repetitive data entry into legacy web forms that have no API.

The expensive reasoning happens once, at design time, and produces a graph. The
graph is data. Running it is cheap, replayable, and observable.

## Two languages, one seam

| Phase | Language | Job |
|---|---|---|
| Design time | AssemblyScript | Author the graph. `asc --noEmit` proves it is well-formed before anything runs. Running the module emits the IR. |
| Run time | Rust → WASM | Evaluate the IR. Hosts the JSON query engine and the JTD validators. |

The seam between them is the IR, and it is a JSON document.

```text
acme-motor-quotes.ts ──asc──> .wasm ──run once──> emit(ptr,len) ──> ….ir.json
                                                                      │
                                            ┌─────────────────────────┘
                                            v
                                 [ evaluator.wasm (Rust) ]
                                            │  one envelope ABI
                            ┌───────────────┴───────────────┐
                            v                               v
                    host-node (CLI)                 host-extension (MV3)
                    files · JSON log                storage.session · IndexedDB
                            │                               │
                            └───────> digital twin <────────┘
```

The same `evaluator.wasm` runs in both hosts, and the same Rust code runs
natively under `cargo test`. A CLI test therefore exercises the real code path
rather than a simulation of it.

## What runs today

`npm test` builds everything and drives the alpha graph against the twin,
offline and with no live model:

```text
53 tests — 5 schema, 8 twin, 31 evaluator (Rust), 9 end-to-end
run ends: success — awaiting_human_final_submit
oracle:   17 writes, all applied · 2 page advances · 17 checkpoints
          1 control cleared before writing (the pre-filled one)
          2 coordinate resolutions (the two controls with no accessible name)
          4 artifacts (2 raw captures, 2 normalized)
```

The same run twice in a row produces an identical action log and an identical
call sequence, because identity is minted deterministically.

## The alpha scenario

**Acme Motor Quotes** is an invented UK car-insurance comparison site, written
from scratch for this repository. It exists to be difficult: a multi-step quote
form with randomised element ids, nested legacy markup, and controls that no
accessible-name lookup can resolve.

A submission JSON carries driver, vehicle, and cover details. The alpha graph
loads it, validates it, fills the form, verifies each write against a fresh
observation, measures remaining work, and stops.

It stops at `awaiting_human_final_submit`. There is no tool that submits, in
any host, at any milestone. Absence is the control.

## Composition, not a script

The graph composes independent nodes. Capture does not know which model will
read its output. Image transformation does not know where the image goes. A
model adapter never chooses the next node. A leaf tool never owns control flow.

This is what makes a binding swappable: replacing one vision model with another
compatible one is a configuration change, not a graph rewrite.

## Layout

```text
threadbox/
  schemas/      JTD (RFC 8927) sources — the specification spine
  guest/        AssemblyScript authoring DSL
  evaluator/    Rust crate -> evaluator.wasm, plus native tests
  host-node/    CLI host implementing the ABI
  extension/    Manifest V3 host implementing the same ABI
  tools/        allowlisted MJS micro-tools
  harness/twin/ Acme Motor Quotes and its action-log oracle
```

Read `ABI.md` for the boundary, `IR.md` for the graph, `DSL.md` for the
authoring vocabulary, `AGENTS.md` for conventions and verification.

## Commands

```sh
npm test                 # codegen, schemas, twin, evaluator, build, validate, e2e
npm run build            # codegen -> evaluator.wasm -> graph.wasm -> graph.ir.json
npm run validate         # run the IR validators over the emitted graph

# drive it by hand against a running twin
node harness/twin/server.mjs 3456 &
node host-node/run.mjs --strict
curl -s localhost:3456/log     # the oracle: what actually happened
```

Each stage communicates through files and exit status, so any of them can be
run alone. `tools/record-corpus.mjs` regenerates the model fixture corpus.

## Model providers

Model invocation is one generic node. Providers are configuration.

In CI the only registered binding is `fixture`, which answers from a
content-addressed corpus keyed by the hash of a canonicalised request. Runs are
therefore deterministic and offline.

Providers are allowlisted by data-residency policy. **A provider whose serving
path is not on the allowlist is prohibited, and no run may route to it.** This
is a configuration gate enforced before dispatch, not an instruction in a
prompt. See `AGENTS.md`.

## Deliberately absent

- No agent that hardcodes capture → model → click.
- No ambient filesystem, network, DOM, or browser API for the DSL or a model.
- No arbitrary JavaScript supplied by a model.
- No unbounded loops.
- No final-submit capability.
- No live provider binding in this milestone.
