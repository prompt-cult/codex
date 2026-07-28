# AGENTS.md — ThreadBox

Conventions and verification commands for anyone, human or model, editing
anything under `threadbox/`. Read `README.md` first for what the system is,
`DSL.md` for the language, `IR.md` for the graph.

## Verification commands

There is no `xmake` here. These are the only gates:

| Scope | Command | Run from |
|---|---|---|
| Guest sources type-check | `npx asc assembly/ir.ts --noEmit` (and once per source) | `threadbox/guest` |
| Guest golden tests | `npm test` (i.e. `node --test tests/**/*.test.js`) | `threadbox/guest` |
| Rust crates | `cargo test` | `threadbox/rust` |
| Runner end to end | `cargo run --bin tb-run -- ../guest/examples/grants-entry.ts` | `threadbox/rust` |
| Harness | `npm test` | `threadbox/harness` |

Do not filter this output. If it is too large, narrow the target — one guest
source, one crate with `-p`, one test with `--test` — rather than piping through
`head`, `tail`, or a pattern match. Filtering hides the unexpected, which is the
only thing worth running a test for.

## Non-negotiable constraints

- **No framework.** Rust standard library plus `wasmi`. No async runtime, no
  serialization framework, no argument parser. Argument handling is a match over
  `std::env::args`.
- **No new guest dependency.** `guest/package.json` pins `assemblyscript` and
  nothing else. The library is standard-library-only AssemblyScript.
- **`wasmi` only in the runner.** `threadbox-ir` and `threadbox-design` have no
  dependencies at all.
- **Nothing in `codex-rs` changes.** `threadbox/rust` is a standalone workspace.
  Copying it to an empty directory and running `cargo test` must work unedited.
- **Unix composition.** Every tool reads a file or standard input and writes
  standard output. Graphs move between stages as bytes. No tool reaches into
  another tool's internals.
- **No legacy path.** There is no compatibility shim for anything that existed
  before the `20260727_pivot` tag. If you find a reference to
  `threadbox.graph.v1`, `NodeHandle`, `Uni<T>`, `Flow`, `threadbox_callback_index`,
  `sdk/`, or `eval-harness/`, it is a defect: delete it, do not adapt it.
- **No unbounded agent chains.** Every `Prompt` is a single model invocation.
  The only repetition is `Retry` with a literal bound, or `ForEach` over a
  finite array. The flatMap-equivalent reshape requires a literal
  element-count bound.
- **No reactive-stream vocabulary.** New data-flow constructs are named
  `project`, `select`, and `expand` (see `DSL.md`), not `map`/`filter`/
  `flatMap`. Names like `Stream`, `Observable`, `Flow`, `Pipeline`, `Uni`, or
  `Multi` (beyond the existing `Multi.over(...)` entry point) are forbidden
  for new constructs, for the same reason `Uni<T>` is already denylisted
  above: this is not a reactive-programming system.

## Documentation before code

Markdown-driven development, in this order:

1. GitHub issue: problem and goals, what and why, never how.
2. `README.md` — user-visible behaviour.
3. `AGENTS.md` — this file, when a convention changes.
4. `DSL.md` / `IR.md` — when the language or graph schema changes.
5. Code.
6. Tests.
7. Re-read the documents and confirm they still describe reality.

A construct that is not in `DSL.md` does not exist. A node field that is not in
`IR.md` does not exist. The documents are the specification, not a description of
the code.

## The boundary is one row

| Module | Function | Parameters | Returns |
|---|---|---|---|
| `threadbox.ir.v1` | `emit` | `ptr: usize, len: i32` | `void` |

Consequences that must not be rediscovered:

- A node identifier is an index into the guest's own arena. It is meaningful only
  inside the serialized graph. Do not invent opaque, generational, or tagged
  handles.
- A callback is an ordinary AssemblyScript function, invoked at graph
  construction time. Table-index dispatch is not part of this system.
- The guest exports `main` and nothing else — no table, no extra start function.
- The host provides no clock, randomness, filesystem, or network. A guest that
  needs input declares it as a named document in the graph.

Adding a capability means bumping `threadbox.ir.v1` to `v2`, never editing the
row.

## Guest style

- AssemblyScript subset only: no function overloading (use optional parameters),
  no string unions, no object literals, no closures.
- Callbacks passed to `.branch()` and `.forEach()` are **named top-level
  functions**. A closure capturing enclosing scope is a grader failure, not a
  style preference: named functions are what make "does this terminate" a
  structural question.
- Every `.attempts(n)` bound is an integer literal.
- Exactly one `.publish()` per program. The guest asserts this before emitting.
- Documentation comments use `///`.
- No magic numbers: use the named constants in `models.ts`.
- `.asText()` / `.asJSON()` is the mandatory finalizer on every model-call
  builder (`ask()` / `askName()`). Omitting it is a compile-time type error,
  not a runtime check.
- Transform expressions (`project`/`select`/`expand`) are single string
  literals passed through to the host unexamined. The guest must not parse,
  compose, or concatenate a jq/xq expression at construction time.
- New discriminators (`promptKind`, `responseKind`, the transform op name)
  are named constants in a `models.ts`-style module — no magic strings in a
  node-construction call.
- There is no dedicated "checkpoint" or "memento" node. A `Step` held in a
  named `const` is already a reusable handle to an earlier point in the
  graph — see the worked example in `DSL.md`. Do not add an identity node
  whose only purpose is to be referenced later.

## Rust style

- Data first: plain structs and enums, exhaustive `match`, no trait hierarchies,
  no wildcard match arms over closed sets.
- Inline `format!` arguments: `format!("{count} nodes")`, not
  `format!("{} nodes", count)`.
- Modules stay under 500 lines excluding tests. Add a module rather than growing
  one.
- The reader in `threadbox-ir` is deliberately tactical and hand-rolled. It is
  regenerated when the graph changes. **No bidirectional round-trip is
  promised** — do not add one, and do not reach for a serialization crate to
  "clean it up".
- Public API entry points validate inputs and return descriptive errors: what
  constraint was violated, the actual value, and what was expected. `assert!` is
  for internal invariants only.
- jq/xq evaluation is a host-side concern, exactly like model resolution and
  named-document loading. `threadbox-ir` validates a `Transform` node's `expr`
  only for non-emptiness at parse time; it never adds a jq engine dependency,
  and the crate stays zero-dependency regardless of how many transform-shaped
  node kinds the DSL grows.

## Error message standard

Name the constraint, the offending value, and the expectation.

Good:

```
graph has 2 Publish nodes at indices [7, 12]; exactly one is required
Retry at index 4 has bound 0; bound must be a positive integer literal
ForEach at index 9 references body index 31 but the arena holds 14 nodes
```

Bad: `invalid graph`, `bad value`, `validation failed`.

## Logging

- Rust: write diagnostics to standard error, the graph to standard output. A tool
  that pollutes standard output is unusable in a pipe.
- `tb-design` appends one JSON line per attempt to its log. Never truncate the
  log to make output prettier; every attempt is evidence.
- Harness JavaScript: report per driver, one line per driver, and distinguish
  `OK`, `SKIP` (local daemon absent), `RATE_LIMIT`, and `FAIL`. A rate limit is
  not a test failure.

## Environment facts

- Secrets live in `/Users/Shared/codex/.env`, gitignored. Load with Node's
  built-in `process.loadEnvFile`. Do not add a dotenv dependency.
- `OPENCODE_API_KEY` serves both OpenCode Zen and OpenCode Go.
  `MISTRAL_API_KEY` and `GROQ_API_KEY` are as named. Ollama needs no key and
  lives at `http://localhost:11434/v1`.
- The authenticated OpenCode Zen catalog does not expose the free-tier model the
  public catalog advertises. Never configure a free model as a required smoke
  target.
- Reasoning models on Zen reject `temperature`. Omit the field; do not default
  it. This is what `supportsTemperature: false` in a catalog entry means.
- Local Ollama has `gemma4:26b` pulled. It is the eco-tier summarize path.
- Auth headers are per adapter — `x-api-key` for Anthropic Messages,
  `x-goog-api-key` for Gemini, `Authorization: Bearer` only for OpenAI-shaped
  endpoints. Never reintroduce a blanket `Authorization` header in shared HTTP
  code.
- This repository has no `Ready` label. Use `enhancement`.

## Commits

- Start with `Issue #<n> <short description>`. No type prefix like `feat:` or
  `Bug:`.
- Say what was achieved and how to verify it: the command to run and the
  expected result.
- No promotional footer. No `Co-Authored-By`.
- Never bypass the pre-commit hook.
- Keep commits atomic. Tidy-up is a separate commit with its own issue; if it
  becomes necessary mid-stream, commit progress as `wip: <n> ...` first.
- Never commit a failing test, dead code, or a disabled feature.
