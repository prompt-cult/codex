## Status of this document

This supersedes #3 and #4 in full. Everything committed under `threadbox/`
before the `20260727_pivot` tag is abandoned, not refactored: the pre-pivot tree
put agent-graph construction on the host side of the WebAssembly boundary, and
every subsequent decision inherited that mistake. The tag preserves it for
archaeology. The branch is reset behind it.

This is the complete and final specification for the rebuild. It is written to be
read once, in order, by someone with no memory of the pre-pivot tree.

---

## 1. What is being built

A system that turns a natural-language description of a repetitive task into a
**reviewable, replayable plan**, and keeps the act of producing the plan strictly
separate from the act of running it.

The plan is a directed acyclic graph — an intermediate representation. It is
produced by compiling a small AssemblyScript program to WebAssembly and running
it in a memory-capped sandbox, where its only effect is to serialize the graph it
built. Nothing in the graph executes during production.

Three artifacts, in dependency order:

1. A **domain-specific language**, embedded in AssemblyScript, whose only purpose
   is to construct the graph. Compiling it proves it is well-formed at the type
   level; running it produces the graph.
2. A **design loop**: a command-line tool that drives a model until the
   AssemblyScript it writes type-checks, logging every attempt.
3. A **runner**: a command-line tool that executes the compiled module under
   `wasmi`, captures the emitted graph, and validates it structurally.

Executing the graph is explicitly not in this specification.

## 2. Why this shape

**The boundary must be narrow.** If the host mints graph nodes, then the set of
combinators is part of the application binary interface, and every change to the
vocabulary is a versioned migration. If the guest builds the graph in its own
linear memory and hands over only the serialized result, then the stable surface
is one function and the vocabulary is a library.

**A serialized plan can be judged before it is trusted.** "Does this terminate",
"does this have exactly one terminal", "is every node reachable" are properties of
a graph, answerable by walking it. They are not properties we assume because the
generator seemed competent. Compiling successfully earns a program the right to be
inspected, nothing more.

**Repetition is the economic case.** An agent that rediscovers the same form
layout on every run pays for that reasoning every run. Work that repeats hundreds
of times against an unchanging interface wants the reasoning done once and the
result replayed. That is what a plan is. It also settles which combinators exist:
only those the repetition needs.

**Separation is what makes either half testable.** Producing a plan has no side
effects, so it can be graded in bulk. Running a plan is where all the risk lives,
so it can be sandboxed and budgeted on its own terms.

## 3. Non-negotiable constraints

- **No framework.** Rust standard library plus `wasmi`. No async runtime, no
  serialization framework, no argument parser. Argument handling is a match over
  `std::env::args`.
- **No new package dependency in the guest.** The AssemblyScript library is
  standard-library-only AssemblyScript.
- **Unix composition.** Every tool reads from a file or standard input and writes
  to standard output. Graphs move between stages as bytes on a pipe. No tool calls
  another tool's internals.
- **Incremental proof.** Each stage in section 10 is independently runnable and
  independently verifiable from a terminal. No stage may depend on a later stage
  existing.
- **Nothing in `codex-rs` changes.** The Rust for this lives in its own workspace
  and must stay extractable to its own repository without edits.
- **No legacy path.** There is no compatibility shim for the pre-pivot surface. No
  file, type, or import name is carried forward for the sake of continuity.

## 4. Repository layout

```
threadbox/
  README.md                 user-facing: what it is, how to run it
  AGENTS.md                 agent-facing: conventions, verification commands
  DSL.md                    the language: every construct, worked example
  IR.md                     the graph: node schema, serialized form, validators
  guest/
    package.json            pins the AssemblyScript compiler, nothing else
    assembly/
      ir.ts                 node structs, arena, builder methods
      emit.ts               arena walk -> serialized bytes, the single import
      models.ts             model-selection builder (inert records)
    examples/               reference programs, one per scenario
    tests/                  golden tests: hand-built arena -> expected bytes
  rust/
    Cargo.toml              standalone workspace, not a codex-rs member
    ir/                     graph structs, reader, writer, validators
    design/                 bin: tb-design
    runner/                 bin: tb-run
  harness/
    package.json            pins the eval driver
    catalogs/               versioned per-driver model catalogs
    policy.yaml             operator-owned tier x role mapping
    providers/              policy resolution, wire adapters, smoke checks
    prompts/                scenario prompts
    graders/                type-check -> structure -> lint gate
    scenarios/              per-scenario expected structure
```

Two package manifests, deliberately: the compiler version that decides "is this
valid AssemblyScript" and the tooling that decides "how do we run the evaluation
matrix" move on different schedules and must not pin each other.

## 5. The boundary

Exactly one import crosses from guest to host.

| Module | Function | Parameters | Returns |
|---|---|---|---|
| `threadbox.ir.v1` | `emit` | `ptr: usize, len: i32` | `void` |

The guest calls it once, at the end of `main`, with a pointer to the serialized
graph and its byte length. The host reads that many bytes from the guest's linear
memory and the guest is finished.

Consequences, stated so they are not rediscovered:

- Handles do not cross the boundary. There are no opaque node identifiers to keep
  generational or tagged. A node identifier is an index into the guest's own arena
  and is meaningful only inside the serialized graph.
- Callbacks do not cross the boundary. A callback is an ordinary AssemblyScript
  function invoked at graph-construction time. Table-index dispatch is not part of
  this system.
- The guest exports `main` and nothing else. There is no exported table, no
  exported start function beyond the standard one.
- The host provides no other capability. No clock, no randomness, no filesystem,
  no network. A guest that wants input receives it as a named document declared in
  the graph and supplied by whoever eventually runs the graph.

Any change to the row above is a breaking change and takes a version bump in the
module name, never an in-place edit.

## 6. The language

Scope: repetitive data entry into a browser-based application that offers no
programmatic interface. Take a structured submission, drive a form, verify each
step, retry on failure, escalate to a more capable model when retrying does not
help, branch on what is actually on screen, and iterate over the submission's
fields.

Every construct below exists because that scenario needs it. Nothing else exists.

| Construct | Node | Meaning |
|---|---|---|
| `loadJson(name)` | `LoadJson` | A named input document, supplied from outside. The guest has no filesystem. |
| `screenshot()` | `Screenshot` | Capture the current view. |
| `.scale()` | `Scale` | Normalize to a fixed width with padding; coordinates map back outside the graph. |
| `.locate(description, model?)` | `Locate` | Resolve a description of a control to viewport coordinates. |
| `.click()` | `Click` | Act at resolved coordinates. |
| `.type(value)` | `Type` | Enter a value at resolved coordinates. |
| `.confirm(assertion, model?)` | `Verify` | Second-opinion check yielding a boolean. |
| `.attempts(n)` | `Retry` | Bounded repetition. `n` is a literal. |
| `.orElse(alternative)` | `Fallback` | The escalation ladder. |
| `.branch(whenTrue, whenFalse)` | `Branch` | The only conditional. |
| `Multi.over(array).forEach(f)` | `ForEach` | Iteration bounded by the input array. `f` is a named top-level function. |
| `.then(next)` / `.after(prior)` | edge | Sequencing. Not a node. |
| `.publish()` | `Publish` | The single terminal. Exactly one per program. |

Two properties follow from the table and matter more than the table:

- **No combinator can produce a back edge.** Repetition is only ever `Retry` with
  a literal bound or `ForEach` over a finite input. Termination is therefore a
  structural question, decidable by walking the graph.
- **No combinator can inspect a value.** `Branch` consumes a `Verify`, which is a
  node, not a host round-trip during construction. The graph is a plan, not a
  partially-evaluated execution.

Style rules the grader enforces, which are ordinary style rules and not boundary
workarounds: callbacks are named top-level functions, never closures capturing
enclosing scope; every `Retry` bound is a literal; there is exactly one `publish`.

## 7. Model selection

A separate builder, in `guest/assembly/models.ts`, producing inert records. It
adds no import and no export. Nothing in it opens a connection, reads an
environment variable, or knows a URL.

Two call forms:

- **Tier only** — `withPlanMode(Tier.Performance)` — defers entirely to the
  operator: whatever the operator currently calls the performance planner.
- **Explicit** — any subset of vendor, model, reasoning effort, context window,
  and driver — constrains exactly the axes named and nothing else.

The record has seven slots: role, tier, vendor, model, think, context window,
driver. Absent slots are omitted from the serialized form and impose no constraint.

Resolution happens outside the guest, against two files with different owners:

- A **driver catalog** per driver, carrying a version, stating what that driver
  currently serves. Replaced wholesale when models are added or retired.
- A **policy** file, operator-owned, mapping logical tier and role to a catalog
  entry.

A tier-only record takes the policy path. Any explicit field takes a filter path,
where zero matches and ambiguous matches are both hard configuration errors.
Because the mapping lives outside the guest, retiring a model is a catalog edit
and never a change to any program.

Resolution precedence, highest first: a forced single model; a per-role override;
a selected tier; the policy's default tier. Unknown, disabled, incompatible, or
over-budget selections fail before any network call, including when forced.
Generated code cannot alter this.

Roles are plan, code, review, summarize. Tiers are eco, balanced, performance.

## 8. The serialized graph

A flat array of nodes plus a header. Each node carries its arena index, its kind,
its parent indices, and kind-specific fields. Edges are parent references only;
there is no separate edge list.

The serialized form is a debugging and review format. It is written by a
hand-rolled walker in the guest and read by a hand-rolled reader in Rust. **No
bidirectional round-trip is promised.** The reader is tactical and is regenerated
when the graph changes. Reading a serialized graph back into a runnable structure
is out of scope.

Four validators, each a walk:

1. Exactly one `Publish`, and it is reachable from every other node's descendant
   chain.
2. No orphans: every node is reachable from the terminal by following parent
   edges.
3. Every `Retry` bound is a positive integer literal.
4. Every `ForEach` body is present and reachable.

A graph that fails any validator is not run. A graph that passes is eligible to be
reviewed, which is a separate judgement.

## 9. The Rust

A standalone workspace at `threadbox/rust/`. Not a member of `codex-rs`: it does
not want that workspace's lockfile, lint matrix, or crate-naming convention, and
it must stay extractable.

| Crate | Kind | Role |
|---|---|---|
| `threadbox-ir` | library | Node structs mirroring the guest's one-for-one; reader and writer; the four validators. |
| `threadbox-design` | binary `tb-design` | Drives the design loop. |
| `threadbox-runner` | binary `tb-run` | Instantiates the module under `wasmi`, implements `emit`, validates, prints. |

`wasmi` is the only dependency, and only in the runner.

`tb-design` owns three things and no others: constructing the invocation,
enforcing an attempt cap, and appending every attempt to a log. It does not parse
AssemblyScript and it does not talk to a model provider directly.

## 10. Stages

Each stage is verifiable from a terminal on its own. No stage may be started
before the previous one verifies.

| # | Stage | Verified by |
|---|---|---|
| 1 | Documentation: `README.md`, `AGENTS.md`, `DSL.md`, `IR.md` written before any code. | Every construct in section 6 and every node field in section 8 is documented. |
| 2 | `guest/assembly/ir.ts`: node structs, arena, builder methods. | Type-checks with no emit. |
| 3 | `guest/assembly/emit.ts`: arena walk to bytes, single import declaration. | Golden test: hand-built arena produces expected bytes. |
| 4 | `guest/assembly/models.ts`: the selection builder. | Type-checks; serialized records match the seven documented slots. |
| 5 | A reference program for the scenario in section 6. | Type-checks; uses only documented constructs. |
| 6 | `threadbox-ir`: structs, reader, four validators. | Crate tests pass, including a case that trips each validator. |
| 7 | `tb-run`: compile, instantiate, capture, validate, print. | Run against the reference program; a valid graph is printed. |
| 8 | `tb-design`: the convergence loop plus attempt log. | Converges on one scenario, or exits nonzero with a complete log. |
| 9 | Harness: catalogs, policy, provider adapters, graders, scenarios. | Unit tests pass; every configured driver reports a result; the reference program grades clean. |

## 11. Definition of done

- Every guest source type-checks with no emit.
- Every Rust crate's tests pass.
- The runner, given the reference program, prints a graph that passes all four
  validators.
- The design loop produces a converged program and a complete attempt log for at
  least one scenario.
- `DSL.md` documents every construct in section 6 with a worked example, and
  `IR.md` documents every node field in section 8.
- The working tree under `threadbox/` is clean.
- No file under `threadbox/` references any pre-pivot type, module, or import name.

## 12. Out of scope

Executing the graph. Any encoding beyond the single documented serialized form.
Wiring real vision or audio transports. Reading a serialized graph back into a
runnable structure. Persistence, scheduling, or a long-running daemon. Any change
to `codex-rs`.
