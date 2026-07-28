# schemas — the specification spine

RFC 8927 JSON Type Definition sources. Every boundary that carries data is
described here and validated in both directions.

A file at `schemas/<path>/<version>.jtd.json` defines the schema URI
`agent-dsl://schemas/<path>/<version>`. Versions are semver and independent:
JTD deliberately has no inheritance or composition, so sections are named and
validated separately rather than faked with an `allOf` hierarchy.

## What is here, and what is not

JTD covers the **data plane** — envelopes, tool inputs and results, provider
outputs, artifact metadata, and the business submission.

The **IR is not validated by JTD**. `IR.md` specifies structural validators
that produce messages naming the constraint, the offending node, and the
expectation:

```text
Loop at index 5 (each-field) has max 0; max must be a positive integer literal
```

A JTD can only say that a value failed at a path. For a document authored by a
model and read by a human reviewer, the better message is worth more than the
uniformity, so the IR validators are hand-written in the evaluator and `IR.md`
is their specification.

## Code generation

`npm run codegen` runs [`jtd-codegen`](https://github.com/simbo1905/jtd-wasm)
over every `*.jtd.json`:

| Target | Output | Consumer |
|---|---|---|
| `js` | `tools/generated/<name>.mjs` | host and MJS tool boundary |
| `rust` | `evaluator/src/generated/<name>.rs` | compiled into `evaluator.wasm` |

Each generated module exports one `validate` function. Generated code is a
build artifact: it is gitignored, never hand-edited, and regenerated from the
schema. Editing generated output instead of its schema is a defect.
