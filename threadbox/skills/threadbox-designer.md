# ThreadBox Designer Skill

You are writing an AssemblyScript program that builds a ThreadBox IR graph.
The program must compile with `asc` (AssemblyScript compiler 0.28.20, pinned in
`guest/package.json`) using `--noEmit`. Read `DSL.md` at the repository root for
the full language reference and `guest/examples/grants-entry.ts` for a complete
worked example before writing anything.

## Imports (use exactly these, add no others)

```typescript
import { loadJson, screenshot, Multi, Step, FieldRef, Doc } from "../assembly/ir";
import { emitGraph } from "../assembly/emit";
import { Spec, Tier, Providers, Model, Think, Context,
         withPlanMode, withBuildMode, withReviewMode, withSummarizeMode } from "../assembly/models";
```

## DSL vocabulary (see `DSL.md` for the full table)

| Construct | Node | Meaning |
|---|---|---|
| `loadJson(name)` | `LoadJson` | A named input document supplied from outside. |
| `screenshot()` | `Screenshot` | Capture the current view. |
| `.scale(width)` | `Scale` | Normalize to a fixed width with padding. |
| `.locate(description, model?)` | `Locate` | Resolve a description to viewport coordinates. |
| `.click()` | `Click` | Act at resolved coordinates. |
| `.type(value)` / `.typeField(ref)` | `Type` | Enter a literal value, or a value read from a document. |
| `.confirm(assertion, model?)` | `Verify` | Second-opinion check yielding a boolean. |
| `.attempts(n)` | `Retry` | Bounded repetition; `n` is an integer literal. |
| `.orElse(alternative)` | `Fallback` | The escalation ladder. |
| `.branch(whenTrue, whenFalse)` | `Branch` | The only conditional. Consumes a `Verify`. |
| `Multi.over(ref).forEach(body)` | `ForEach` | Iteration bounded by an input array. |
| `.then(next)` / `.after(prior)` | edge | Sequencing, not a node. |
| `.publish()` | `Publish` | The single terminal. Exactly one per program. |

## Hard rules

- No closures: callbacks passed to `.branch()` and `.forEach()` must be named
  top-level functions.
- Every `.attempts(n)` bound is an integer literal, never a variable.
- Exactly one `.publish()` per program.
- The last statement of `main(): void` must be `emitGraph();`.
- Export `function main(): void`, and export nothing else.
- Do not add any import beyond the three lines above.
- Do not modify any file other than the target scenario file you were asked to
  write.

## Output

Write the complete program to the target path you were given (an environment
variable `TB_DESIGN_TARGET` names it during automated runs; a human operator
will simply tell you the path). Do not print the program to standard output
instead of writing the file — `tb-design` checks the file on disk, not your
transcript.
