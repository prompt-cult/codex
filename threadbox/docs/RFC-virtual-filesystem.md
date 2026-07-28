# RFC — Bounded virtual filesystem (VFS) for agent workspace

> **Status:** Proposed. Documentation-only artifact. No code, no IR change, no
> test has been written against this RFC. Nothing in `README.md`, `AGENTS.md`,
> `DSL.md`, or `IR.md` has been edited to reflect it yet — those files are
> intentionally left untouched here to avoid colliding with concurrent review
> passes over the codebase. This RFC is the proposal those four documents
> would be updated *from*, per `AGENTS.md`'s Markdown-Driven-Development
> order, once accepted. It is a sibling of `PLAN_DRAFT_agent-chaining.md`
> (the `Prompt`/`Transform` feature): consistent in vocabulary, independent
> in scope, and does not depend on that plan landing first.

## 1. Summary

Extend the ThreadBox DSL/IR with a bounded, host-resolved, **in-memory-only**
virtual filesystem so a graph can save intermediate results to a named path,
reload them, list what exists, and search — without touching a real
filesystem, without new Rust dependencies, and without breaking the
"guest describes, host resolves" static-DAG contract. Three named mount
points back the store: `/app` (read-only, pre-populated), `/workspace`
(read-write, execution-scoped), `/tmp` (ephemeral scratch).

## 2. Motivation

The `Prompt`/`Transform` plan lets a graph chain model calls and reshape
their output, but every intermediate value only ever flows forward through
`parents`. There is no way for a later step to reference an *earlier* named
result except by keeping it reachable through the parent chain, and no way
to list, inspect, or reuse a prompt template except by inlining it as a
literal or a host-resolved name on the `Prompt` node itself. Two needs
follow directly from that gap:

- **Save/reload of agent output.** A long pipeline (summarize → save →
  reload later → escalate) needs a place to put a result down without
  wiring every downstream consumer through the same parent edge.
- **Prompt templates as named, listable, host-owned resources**, rather than
  an opaque host-side lookup keyed by a string the guest cannot enumerate or
  distinguish from a literal.

This is **not** a request for real file I/O. The guest still never touches a
disk, a path traversal, or an OS call. It is a bounded, in-memory key-value
store the host maintains for the duration of one execution, shaped like a
filesystem because that shape is already familiar and already bounded (no
recursion, no cycles, closed set of mount prefixes).

## 3. Non-goals

- Real filesystem access of any kind. No `std::fs`, no `std::path`, no
  path-canonicalization crate.
- Persistence across executions. `/tmp` and `/workspace` are execution-scoped;
  nothing survives a run unless a future, separate feature says otherwise.
- Arbitrary path depth or unbounded listings — every `list`/`glob`/`grep`
  result is bounded by a named constant, mirroring `Transform`'s `expand`
  requiring `maxItems`.
- Symlinks, permissions, timestamps, or any other real-filesystem metadata.
- Bidirectional round-trip of the store's contents into the serialized IR
  format. The store is runtime-only state, not a graph node's payload.

## 4. Design

### 4.1 One new node kind: `FileOp`

| Kind | `parents` | Extra fields | Meaning |
|---|---|---|---|
| `FileOp` | `[source]` (data edge) when `op:"write"`; `[]` or `[seq]` (sequencing-only edge, no data flows) otherwise | `op: "read"\|"write"\|"list"\|"edit"\|"glob"\|"grep"` + per-op fields below | A host-resolved virtual-filesystem operation |

Per-op fields:

| `op` | Required | Optional | Output |
|---|---|---|---|
| `read` | `path`, `contentKind:"text"\|"json"` | `offset?`, `limit?` (text only) | text or JSON |
| `write` | `path` | — (`parents[0]` = source, required) | `{bytesWritten}` |
| `list` | `path` | `recursive?`, `depth?` | `[{name,path,type}]` |
| `edit` | `path`, `oldText`, `newText` | `replaceAll?` | `{replacements}` |
| `glob` | `pattern` | `path?` | `[path]` |
| `grep` | `pattern` | `path?`, `globFilter?`, `caseSensitive?` | `[{path,line,text}]` |

**Why one kind, not seven.** This mirrors `Transform`'s `op` discriminator
exactly: one IR row, one validator block, one emit branch, shared
mount/path checks, instead of seven duplicated everything. Adding an op
later is one discriminator value, not a new kind. It also keeps the "the
twelve original kinds plus a small, closed set of extensions" shape of
`IR.md`'s node-kinds table intact rather than growing it linearly per verb.

### 4.2 VFS content model

The host stores UTF-8 bytes keyed by path. `write` serializes: text
as-is, JSON as minified UTF-8. `read` with `contentKind:"json"` is valid
only when the path was last written from a JSON-producing source — checked
best-effort at the validator stage (see V13b below) and enforced for real
only by the host at execution time, since the validator cannot see runtime
values. `edit` is text-only. `offset`/`limit` are only valid when
`contentKind:"text"`.

### 4.3 Mount points (three, closed set)

| Mount | Access | Lifetime | Purpose |
|---|---|---|---|
| `/app` | read-only | pre-populated before the run starts | prompt templates and other operator-owned named resources |
| `/workspace` | read-write | one execution | agent-produced intermediate and final artifacts |
| `/tmp` | read-write | one execution, ephemeral | scratch space with no expectation of being read back by a later, unrelated run |

No separate `/home` mount — a user-suggested `/home/agent-desk` is
subsumed by `/workspace`; see the open question below if a distinct
lifetime or access policy is actually wanted for that concept. The set of
three mount prefixes is closed and validated at the **validator stage**,
not the parser: the parser only checks shape (a `path` field is present and
is a string), the validator checks semantics (the string names a real
mount, contains no `..` traversal segment).

### 4.4 `step.saveTo(path)`

A `Step` method, mirroring the existing chaining idiom. `parents[0]` is the
producing `Step`'s arena index (not a value — the guest never inspects
what it is saving), `path` is a literal string. This is the same "index in,
never a value" shape as `Type(valueKind:"field")`'s second parent.

### 4.5 Prompt templates as files

`loadPrompt(key)` returns a new lightweight `PromptRef` type — analogous to
`FieldRef` — consumed by `askPrompt(ref, source, model)`, which produces a
`Prompt` node with `promptKind:"name"`. **This is no change to the IR at
all.** `Prompt` already supports a named-template lookup; `PromptRef` is
sugar that lets the guest express "this name came from listing `/app`"
without adding a new node kind. The file-backed template store is a host
implementation detail sitting behind the same `promptKind:"name"`
resolution that already exists.

### 4.6 Sequencing-only edges

Documented here for the first time, though the mechanism already exists
implicitly for `.then`/`.after`: a `FileOp` with `op != "write"` may carry
an optional `parents[0]` meaning "must complete after this, but no data
flows from it." A `read`, `list`, `glob`, or `grep` that must run after some
prior step (for ordering, not data) uses this. The host runner must never
pass a sequencing parent's value as an input to a non-`write` `FileOp`.

### 4.7 `listFiles`/`findFiles`/`searchFiles` → existing `ForEach`

No new combinator. `listFiles(path)`, `findFiles(pattern)`, and
`searchFiles(pattern)` each produce a `FileOp` (`list`/`glob`/`grep`
respectively) whose array output is iterated with the existing
`step.forEach(body)`, `over: "."` — the same convention `Prompt`/`Transform`
sources already use for `ForEach` (see `IR.md`'s "`ForEach` over agent
output"). A new validator (V15 below) requires the `FileOp` in that
position to be one of the three list-shaped ops.

### 4.8 Naming

Verbs are lower-camelCase, consistent with `loadJson`/`.locate`/`.project`,
not the PascalCase `ReadFile`/`WriteFile`/... of the MCP-style tool
signatures this RFC originated from: `loadFile`, `loadJsonFile`,
`step.saveTo`, `listFiles`, `editFile`, `findFiles`, `searchFiles`.

## 5. Constraint compliance

| # | Constraint | Compliance |
|---|---|---|
| No new Rust dependencies | `FileOp` is one enum variant, flat fields, string-prefix/substring checks only |
| No real filesystem | Host in-memory `HashMap<String, Vec<u8>>`; no `std::fs`/`std::path` |
| Opaque values to guest | `saveTo`'s content flows from a parent node, never inspected by the guest; `editFile`'s `oldText`/`newText` are literal strings the guest supplies, replacement logic is host-side |
| No reactive-stream vocabulary | No `Stream`/`Flow`/`Observable`; plain `Step`-returning functions |
| Static DAG, no live-value branching | Every `FileOp` parent is an arena index; sequencing edges carry no data |
| Host resolves / guest describes | Identical pattern to `LoadJson` named documents and `Prompt` named templates |

## 6. Worked example

```mermaid
graph LR
  N0["0 LoadJson name=ticket"] --> N1["1 Prompt literal/json summarize"]
  N1 --> N2["2 FileOp write /workspace/summary.json"]
  N2 -.->|sequencing, no data| N3["3 FileOp read/json same path"]
  N3 --> N4["4 Prompt name/text escalate template"]
  N4 --> N5["5 Publish"]
```

Node `2`'s edge from node `1` is a **data** edge (the summary's JSON is what
gets written). Node `3`'s edge from node `2` is a **sequencing-only** edge —
`read` needs the `write` to have happened first, but does not consume its
`{bytesWritten}` output. This is the first place either document would make
the data-edge-vs-sequencing-edge distinction explicit for a non-`.then`
combinator.

## 7. Proposed validators (additive, V10–V15)

Following the existing convention (`IR.md`'s validators 5–9 are additive and
never re-examine a pre-existing node kind), a `FileOp`-aware validator set
would be entirely new numbers appended after the existing nine:

| # | Property |
|---|---|
| V10 | Every `FileOp` has the fields its `op` requires (per the table in §4.1). |
| V11 | Every `path`/`pattern`'s optional `path` names one of the three mount prefixes and contains no `..` segment. |
| V12 | No `write` or `edit` targets `/app` (read-only). |
| V13 | Every `write`'s source (`parents[0]`) is a data-producing kind. V13b (best-effort, static only): a `read` with `contentKind:"json"` should be checked against the last known producer of that path when statically determinable; this is documented as a static, non-exhaustive check, not a runtime guarantee. |
| V14 | Every `edit` has both `oldText` and `newText`. V14b: `edit` is rejected if `contentKind` would resolve to non-text. V14c: `offset`/`limit` are rejected when `contentKind:"json"`. |
| V15 | A `ForEach` sourced from a `FileOp` requires that `FileOp`'s `op` to be `list`, `glob`, or `grep`. |

Named constants, mirrored between `fs.ts` (guest) and `lib.rs` (Rust),
cross-referenced by comment so the two cannot silently drift: `MAX_FILE_SIZE`,
`MAX_GLOB_RESULTS`, `MAX_GREP_RESULTS`, `MAX_LIST_ENTRIES`, `MAX_LIST_DEPTH`,
`MAX_PATH_BYTES`.

## 8. Regression watch-list (for whoever implements this)

- Existing golden fixtures must stay byte-identical — `FileOp` is additive
  only; no existing emit branch changes shape.
- Every "unknown kind"/kind-count assertion across `emit.ts`, `ir.ts`,
  `lib.rs`, `parse.rs`, and the four documentation files must be located and
  updated together — the exact current count must be verified at
  implementation time, not assumed from this RFC.
- A validator-ordering test is required: a graph failing both an earlier
  validator (e.g. V3) and a new V1x must report the earlier one first,
  exactly as the existing suite already tests for V3-vs-V5.
- The host runner must never pass a sequencing-parent's value into a
  non-`write` `FileOp` — this needs its own test, since it is the first
  place a parent index exists purely for ordering.

## 9. Open questions

1. Should `glob`/`grep`/`list` require an explicit `maxResults`/`depth` IR
   field (mirroring `expand`'s mandatory `maxItems`), or is a host-side
   named constant sufficient?
2. Should `editFile` eventually accept a `FieldRef`-sourced replacement
   value (mirroring `Type.valueKind:"field"`), or is literal-only text
   sufficient for now?
3. Is `contentKind` on `read` mandatory with no default (matching
   `responseKind`'s discipline on `Prompt`), confirmed?
4. Does `/workspace` fully subsume the originally suggested
   `/home/agent-desk`, or is a separate home mount wanted for a distinct
   lifetime or access policy?

## 10. Path to acceptance

This RFC is a proposal only. If accepted, the Markdown-Driven-Development
order in `AGENTS.md` applies from here: a GitHub issue stating only what/why,
then `README.md`, then `AGENTS.md`, then `DSL.md`, then `IR.md`, then a
`PLAN_<issue>.md` with the exact Phase 2+ per-file change list (guest
`fs.ts`, `ir.ts`, `emit.ts` and its golden fixture; Rust `lib.rs` enum,
`parse.rs`, `validate.rs` for V10–V15; and the corresponding Rust tests,
including the validator-ordering test above) — only after those four
documents are updated to describe the accepted shape, per the existing
convention this repository already follows for `Prompt`/`Transform`.
