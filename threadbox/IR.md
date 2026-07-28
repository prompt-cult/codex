# IR.md — the graph

The intermediate representation is a flat array of nodes plus a header. Each node
carries its arena index, its kind, its parent indices, and kind-specific fields.
**Edges are parent references only; there is no separate edge list.**

The serialized form is a debugging and review format. It is written by a
hand-rolled walker in the guest (`guest/assembly/emit.ts`) and read by a
hand-rolled reader in Rust (`rust/ir/src/lib.rs`). **No bidirectional round-trip
is promised.** The reader is tactical and is regenerated when the graph changes.
Reading a serialized graph back into a runnable structure is out of scope.

## Envelope

```json
{
  "ir": "threadbox.ir.v1",
  "nodes": [ ]
}
```

| Field | Type | Meaning |
|---|---|---|
| `ir` | string | Format identifier. Must equal `threadbox.ir.v1`. A reader that sees anything else stops. |
| `nodes` | array | Every node in the arena, in ascending index order. `nodes[i].i == i`. |

The envelope carries no node count and no pointer to the terminal. Both are
derivable by walking, and a derivable field that is also stored is a field that
can disagree with itself.

## Common node fields

Every node object carries these three, in this order:

| Field | Type | Meaning |
|---|---|---|
| `i` | integer | Arena index. Equals the node's position in `nodes`. Meaningful only inside this graph. |
| `kind` | string | One of the fourteen kinds below. |
| `parents` | array of integer | Indices this node consumes. Order is significant and fixed per kind. |

Kind-specific fields follow, in the order listed in the table for that kind.
Absent optional fields are omitted entirely rather than emitted as null.

## Node kinds

| Kind | `parents` | Extra fields | Meaning |
|---|---|---|---|
| `LoadJson` | `[]` | `name: string` | A named input document, supplied from outside. Always a root. |
| `Screenshot` | `[]` | — | Capture the current view. |
| `Scale` | `[source]` | `width: integer` | Normalize to `width` pixels with padding. |
| `Locate` | `[image]` | `description: string`, `model?: ModelSpec` | Resolve a description to viewport coordinates. |
| `Click` | `[target]` | — | Act at the coordinates resolved by `target`. |
| `Type` | `[target]` or `[target, doc]` | `valueKind: "literal" \| "field"`, `value: string` | Enter a value. |
| `Verify` | `[image]` | `assertion: string`, `model?: ModelSpec` | Second-opinion check yielding a boolean. |
| `Retry` | `[guarded]` | `bound: integer` | Repeat `guarded` at most `bound` times. `bound` is a positive integer literal. |
| `Fallback` | `[primary, alternative]` | — | Use `alternative` when `primary` does not succeed. |
| `Branch` | `[condition, whenTrue, whenFalse]` | — | `condition` must be a `Verify`. The only conditional. |
| `ForEach` | `[source]` | `over: string`, `body: integer` | Iterate the body subgraph ending at `body` once per element of the array named by `over` in the document `source`. |
| `Publish` | `[value]` | — | The single terminal. Exactly one per graph. |
| `Prompt` | `[source]` | `promptKind: "literal" \| "name"`, `prompt: string`, `responseKind: "text" \| "json"`, `model: ModelSpec` | Send a prompt to a model and receive text or structured JSON. `source` is `LoadJson`, `Prompt`, or `Transform`. `model` is required (unlike the optional `model?` on `Locate`/`Verify`). |
| `Transform` | `[source]` | `op: "project" \| "select" \| "expand"`, `expr: string`, `maxItems?: integer` | Reshape data with a host-evaluated jq/xq expression. `source` is `LoadJson`, `Prompt`, or `Transform`. `maxItems` is required when `op` is `"expand"`, absent otherwise. |

### Parent order

Parent order is part of the schema and is not negotiable per node:

- `Fallback`: `parents[0]` is the primary attempt, `parents[1]` the escalation.
- `Branch`: `parents[0]` is the `Verify`, `parents[1]` the true arm, `parents[2]`
  the false arm.
- `Type` with `valueKind: "field"`: `parents[0]` is the located target,
  `parents[1]` is the `LoadJson` node the field is read from. With
  `valueKind: "literal"` there is no second parent.
- `Prompt` and `Transform`: `parents[0]` is the data source. Both take exactly
  one parent (plus the usual optional trailing sequencing parent).
- `ForEach`: `parents[0]` was always the document a named array field is read
  from; it may now also be a `Prompt` (with `responseKind: "json"`) or a
  `Transform` — see "`ForEach` over agent output" below.
- Every other kind takes exactly the parents listed, in that order.

### Sequencing edges

`.then(next)` and `.after(prior)` are edges, not nodes. Both append **one trailing
parent** to the *root* of the following chain — the first node built in that
chain, not its tail — after that kind's own parents.

The distinction matters: `a.then(screenshot().scale(1280))` attaches `a` to the
`Screenshot`, giving it `[a]`, and leaves `Scale` as `[source]`. Attaching to the
tail instead would give `Scale` a second parent and contradict its row above.

So every kind's `parents` may carry exactly one extra trailing index when it is a
chain root reached by a sequencing edge. `Screenshot` is the usual case, since a
chain normally starts with one.

### `ForEach.body`

`body` is an arena index, **not** a member of `parents`. It names the **tail** of
the body subgraph.

The body is self-contained: it is built before the `ForEach` node, starts its own
chain, and its only connection to the surrounding graph is through the `ForEach`
that names it. Consequently `body` is strictly less than the `ForEach` node's own
index, exactly like a parent, and the graph has no forward edges of any kind.

A walk of the graph must nonetheless traverse `parents` *and* `body`, because the
body is reachable through no other field. That is why "every `ForEach` has a body"
is a validator in its own right rather than a consequence of the orphan check.

`over` is the field path within the document at `parents[0]` — the string given to
`doc.fields(path)`. A `FieldRef` is not a node; it is this string plus that index.

Inside a body, the element currently being iterated is referenced by the path
`<over>[]`. A `Type` node keying the whole element therefore carries
`value: "answers[]"`, and a field of the element carries `value:
"answers[].label"`. The trailing `[]` is what distinguishes an element reference
from the array itself.

### `ForEach` over agent output

`parents[0]` of a `ForEach` may be a `Prompt` (whose `responseKind` must be
`"json"`) or a `Transform`, in addition to the original `LoadJson`. In both
cases `over` is the literal string `"."` — the whole output is the array being
iterated, since a model call or a jq expression's result has no named field
path the way a `LoadJson` document does. Everything else about `body` is
unchanged: it is built before the `ForEach` node, is strictly less than the
`ForEach`'s own index, and is reachable only through that field.

### `Prompt` and `Transform`

`Prompt` sends `prompt` to `model`, with `parents[0]` supplying context (the
document, or a prior model/transform output). `promptKind` records whether
`prompt` is the literal template text or a logical key the host resolves
against operator-owned configuration — the same distinction `Type`'s
`valueKind` already makes for literal-vs-referenced values. `responseKind`
declares whether the caller expects free text or structured JSON back;
unlike `Locate`/`Verify`, `model` is **required**, not optional, because a
`Prompt` has no vision-specific default the way locating a UI element does.

`Transform` records a single jq/xq expression string in `expr` and which of
three shapes it takes in `op`: `"project"` (map-equivalent, reshape each
element), `"select"` (filter-equivalent, keep matching elements), or
`"expand"` (flatMap-equivalent, reshape and flatten, bounded by the required
`maxItems`). The guest never parses, composes, or evaluates `expr` — it is an
opaque string, exactly as a `Locate` description or a `Verify` assertion is an
opaque string a model interprets. The host evaluates it at execution time.

### `ModelSpec`

The `model` field — optional on `Locate` and `Verify`, required on `Prompt` —
is an object with up to seven slots. Absent slots are omitted and impose no
constraint. **Slots are emitted in the order listed below**, so that golden
byte comparisons are meaningful.

| Slot | Type | Values |
|---|---|---|
| `role` | string | `plan`, `code`, `review`, `summarize` |
| `tier` | string | `eco`, `balanced`, `performance` |
| `vendor` | string | who trained the weights |
| `model` | string | a catalog reference, not a wire identifier |
| `think` | string | `none`, `low`, `medium`, `high` |
| `contextWindow` | integer | usable input tokens; omitted when zero |
| `driver` | string | who serves the model |

There are exactly two shapes, and which one a record has is decided by whether
`tier` is present:

- **Policy path** — `role` and `tier`, nothing else. The operator's table answers
  "which model is the balanced reviewer today". `role` is required here, because
  the lookup key is the pair.
- **Catalog filter path** — any of `vendor`, `model`, `think`, `contextWindow`,
  `driver`, and **no `tier`**. The named axes are matched against catalog entries,
  where zero matches and ambiguous matches are both hard configuration errors.
  `role` is omitted, because the record names its model directly and no policy
  lookup happens.

This is why a `Locate` that names a vision model explicitly carries no `role`:
resolving pixels to coordinates is not one of the four logical roles, and the
filter path does not need one.

Resolution happens entirely outside the guest. When `model` is absent altogether,
the operator's configured default for that node kind applies.

## Worked fragment

A document, a screenshot, a scale, a locate, a click, a field-driven type, a retry
bound, and the terminal:

```json
{
  "ir": "threadbox.ir.v1",
  "nodes": [
    { "i": 0, "kind": "LoadJson", "parents": [], "name": "submission" },
    { "i": 1, "kind": "Screenshot", "parents": [] },
    { "i": 2, "kind": "Scale", "parents": [1], "width": 1280 },
    { "i": 3, "kind": "Locate", "parents": [2],
      "description": "the input labelled 'Case Number'",
      "model": { "vendor": "anthropic", "model": "opus-performance",
                 "think": "medium", "contextWindow": 1000000 } },
    { "i": 4, "kind": "Click", "parents": [3] },
    { "i": 5, "kind": "Type", "parents": [4, 0],
      "valueKind": "field", "value": "caseNumber" },
    { "i": 6, "kind": "Retry", "parents": [5], "bound": 3 },
    { "i": 7, "kind": "Publish", "parents": [6] }
  ]
}
```

## `Prompt` and `Transform` fragments

A document, a JSON-producing prompt, a select-shaped transform, and an
expand-shaped transform with its required bound. This is a **field-shape
catalog**, not a complete valid graph on its own — node `4` is not reachable
from `Publish` and exists only to show `expand`'s `maxItems` shape:

```json
{
  "ir": "threadbox.ir.v1",
  "nodes": [
    { "i": 0, "kind": "LoadJson", "parents": [], "name": "ticket" },
    { "i": 1, "kind": "Prompt", "parents": [0],
      "promptKind": "literal",
      "prompt": "List all fields with resolution status as a JSON array",
      "responseKind": "json",
      "model": { "role": "summarize", "tier": "eco" } },
    { "i": 2, "kind": "Transform", "parents": [1],
      "op": "select",
      "expr": ".fields[] | select(.resolved == false)" },
    { "i": 3, "kind": "Prompt", "parents": [2],
      "promptKind": "name",
      "prompt": "support.escalate-unresolved-field.v1",
      "responseKind": "text",
      "model": { "vendor": "anthropic", "model": "opus-performance",
                 "think": "high", "contextWindow": 1000000 } },
    { "i": 4, "kind": "Transform", "parents": [1],
      "op": "expand",
      "expr": ".tags[]",
      "maxItems": 50 },
    { "i": 5, "kind": "Publish", "parents": [3] }
  ]
}
```

Node `1`'s `model` has only `role` and `tier` — the policy path, exactly as
`ModelSpec` already permits. Node `3`'s `model` has no `role` — the catalog
filter path, naming the vendor and model directly, exactly as it already does
for a `Locate` that names a vision model explicitly. Node `4` illustrates
`expand`'s required `maxItems`; nodes `2` and `3` illustrate that `select` and
the catalog-filter form of `Prompt` carry none.

## Validators

Nine validators, each a walk. They run in this order and the first failure stops
the run. A graph that fails any validator is not run; a graph that passes is
*eligible* to be reviewed, which is a separate judgement.

| # | Property | Failure message shape |
|---|---|---|
| 1 | Exactly one `Publish`. | `graph has 2 Publish nodes at indices [7, 12]; exactly one is required` |
| 2 | No orphans: every node is reachable from the terminal by following `parents` and `ForEach.body`. | `node 5 (Screenshot) is unreachable from the Publish at index 11` |
| 3 | Every `Retry` bound is a positive integer. | `Retry at index 4 has bound 0; bound must be a positive integer literal` |
| 4 | Every `ForEach` names a body below its own index. | `ForEach at index 9 has no body; every ForEach must name the tail of its body subgraph` |
| 5 | Every `Prompt` has a `model`. | `Prompt at index 3 is missing required field "model"; every Prompt must name a model` |
| 6 | Every `Transform`'s source is a data-producing kind. | `Transform at index 5 has source node 4 of kind "Click"; source must be LoadJson, Prompt, or Transform` |
| 7 | Every `ForEach` sourced from a `Prompt` requires `responseKind: "json"` on that `Prompt`. | `ForEach at index 7 has source Prompt at index 1 with responseKind "text"; ForEach source Prompt must have responseKind "json"` |
| 8 | Every `Transform` with `op: "expand"` has a `maxItems`. | `Transform at index 4 has op "expand" but is missing required field "maxItems"; expand requires a literal integer bound` |
| 9 | Every `expand`'s `maxItems` is within the operator-configured maximum. | `Transform at index 4 has maxItems 200; maximum permitted is 100` |

Validator 2 runs only once validator 1 has passed, so exactly one `Publish` exists
and its index is the walk root. Validator 4 owns the semantic requirement — a
`ForEach` must have a body, and that body must have been built before it — while
the bounds check belongs to the reader below. Its second message shape is
`ForEach at index 9 names body index 12, which is not below it; a body is built
before the ForEach that names it`.

Validators 5–9 are additive: they constrain only the two new kinds and their
interaction with `ForEach`, and never re-examine a node kind that existed
before this section. `Transform`'s `expr` is validated only for non-emptiness,
never for jq/xq syntactic or semantic correctness — that is the host's concern
at execution time, not this reader's.

Structural invariants the reader enforces before any validator runs, because a
graph that breaks them is malformed rather than invalid:

- `ir` equals `threadbox.ir.v1`.
- `nodes[i].i == i` for every `i`.
- Every parent index and every `body` index is within bounds.
- Every parent index is strictly less than the referencing node's own index. The
  same holds for `body`, but that is validator 4's business rather than the
  reader's, so the reader checks only that the index exists.
- `kind` is one of the fourteen documented kinds.
- `promptKind` is `"literal"` or `"name"`; `responseKind` is `"text"` or
  `"json"`; `Transform`'s `op` is `"project"`, `"select"`, or `"expand"` —
  parsed the same way `Type`'s `valueKind` already is, with an unknown value
  rejected at parse time rather than deferred to a validator.

Taken together the two layers make the graph acyclic by construction: the arena
only ever appends, so a parent edge always points backward; a body is built before
the `ForEach` that names it, so that edge points backward too. No edge of any kind
can point forward, and a walk needs a visited set only to avoid revisiting shared
subgraphs, never to escape a cycle.

Termination is therefore decidable by inspection: the only repetition is `Retry`
with a positive literal bound and `ForEach` over a finite array.

## Changing the schema

The serialized form is not an ABI. Adding a node kind or a field means:

1. `IR.md` first — this table is the specification.
2. The guest writer in `emit.ts`.
3. The golden test bytes in `guest/tests/`.
4. The Rust reader and its tests.

The one thing that is an ABI is the import row in `README.md`. It does not change
here.
