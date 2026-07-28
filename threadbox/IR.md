# IR.md — the graph

The IR is the seam between the AssemblyScript authoring language and the Rust
evaluator. It is a JSON document, it is validated before it runs, and it is the
specification of what a run may do.

## Envelope

```json
{
  "ir": "threadbox.ir.v2",
  "graph": {
    "id": "acme-motor-quotes-alpha",
    "version": "0.1.0",
    "nodes": []
  }
}
```

| Field | Meaning |
|---|---|
| `ir` | Format identifier. Must equal `threadbox.ir.v2`. A reader that sees anything else stops. |
| `graph.id` | Stable graph name, used in logs and checkpoints. |
| `graph.version` | Semver of this graph definition. |
| `graph.nodes` | The arena, ascending. `nodes[i].i == i`. |

## Common node fields

| Field | Type | Meaning |
|---|---|---|
| `i` | integer | Arena index; equals position. |
| `id` | string | Stable human-readable node id, unique within its arena. |
| `kind` | string | One of the ten kinds below. |
| `parents` | array of integer | Indices this node consumes, in significant order. |

Kind-specific fields follow. Absent optional fields are omitted, never null.

## Evaluation context

Every node produces one JSON value. Expressions are evaluated by the bounded
query engine (see `Transform`) against this context:

```json
{
  "parents": [ ],
  "vars": { },
  "memo": { }
}
```

- `parents` — the values of this node's parents, in `parents` order.
- `vars` — loop variables currently in scope.
- `memo` — values of causally reachable earlier nodes, keyed by `id`.

`memo` is what lets a node select work from twenty steps earlier rather than
only from its immediate predecessor.

## Node kinds

| Kind | Extra fields | Produces |
|---|---|---|
| `Input` | `name`, `schemaUri` | The named input document, admitted read-only. |
| `Tool` | `tool`, `input`, `resultSchemaUri?` | The tool's result envelope. |
| `Agent` | `binding`, `prompt`, `input`, `outputSchemaUri` | The provider's structured output. |
| `Transform` | `expr` | The expression's value. |
| `Guard` | `mode`, `expr?`, `schemaUri?` | Its input value, unchanged. |
| `Loop` | `var`, `over`, `max`, `body` | Array of body values, one per iteration. |
| `Branch` | `cond`, `whenTrue`, `whenFalse` | The taken arm's tail value. |
| `Human` | `classification`, `input` | The typed decision. |
| `Checkpoint` | `label`, `input` | Its input value, unchanged. |
| `Terminal` | `status`, `outcome` | The run result. |

### `Input`

Declares a document supplied from outside. The guest has no filesystem; the
host mounts the document read-only under `inputs/`.

### `Tool`

`input` is an expression producing the request body. `tool` is a logical name
resolved by the host registry — the graph never names a module path, a URL, or
a credential.

### `Agent`

One generic model invocation. `binding` names a configured provider record;
swapping it for another compatible binding must not require touching any other
node. `outputSchemaUri` is validated before the value is admitted, so a
malformed model response fails at the edge rather than downstream.

### `Guard`

`mode` is `assert` or `validate`.

- `assert` — `expr` must produce `true`. The guard **passes its input through
  unchanged**; it does not replace the value with a boolean. An assertion is a
  gate, not a projection.
- `validate` — the input is validated against `schemaUri`.

A failed guard fails the transition before the next node runs. This is how an
image edge refuses a raw capture, and how an audio artifact never reaches a
vision binding.

### `Loop`

Bounded repetition. `over` produces an array; each element is bound to `var`
and the body is evaluated once per element.

`body` is a **nested arena** with its own local indices:

```json
{
  "i": 5, "id": "each-field", "kind": "Loop", "parents": [4],
  "var": "field", "over": ".parents[0].fields", "max": 50,
  "body": { "nodes": [] }
}
```

The loop's value is an array of the last body node's values. `max` is a
positive integer literal, so the cost of the loop is bounded before the graph
runs. There is no other repetition construct and no recursion, which is what
makes termination decidable by inspection.

### `Branch`

The only conditional. `cond` is evaluated over values that already exist in
the graph; it never observes a live value during construction, so the graph
stays a plan rather than a partially evaluated execution.

`whenTrue` and `whenFalse` are nested arenas with their own local indices,
exactly like a loop body. Only the taken arm runs, and the branch's value is
that arm's tail value.

```json
{
  "i": 3, "id": "pick-target", "kind": "Branch", "parents": [2],
  "cond": ".parents[0].result.resolved",
  "whenTrue":  { "nodes": [] },
  "whenFalse": { "nodes": [] }
}
```

Running only the taken arm is what makes cost an architectural property rather
than an aspiration: an expensive fallback — a capture, a transform, and a model
call — costs nothing on the fields a cheap deterministic lookup already
resolved.

Neither arm may contain a `Terminal`. A branch chooses what happens next; it
does not decide whether the run is over.

### `Terminal`

Exactly one per top-level arena. `status` is one of `success`, `exhausted`,
`cancelled`, `failure`. `outcome` is a stable string such as
`awaiting_human_final_submit`.

### Not implemented in this milestone

`Parallel` and `Join` are reserved. They are specified here so the IR version
does not have to change when the first independent read-only branch appears,
and the evaluator rejects them today with an explicit "not implemented in
threadbox.ir.v2" message rather than ignoring them.

## Validators

Run in order; the first failure stops the run. A graph that fails is not run.

| # | Property |
|---|---|
| 1 | `ir` equals `threadbox.ir.v2`, and `nodes[i].i == i` in every arena. |
| 2 | Every `parents` index is in bounds and strictly less than the referencing node's own index. |
| 3 | Exactly one `Terminal` in the top-level arena, and none nested in a loop body. |
| 4 | Every node `id` is unique within its arena and non-empty. |
| 5 | Every `Loop` has a non-empty body and a `max` that is a positive integer; every `Branch` has two non-empty arms and no nested `Terminal`. |
| 6 | Every node is reachable from the `Terminal` by following `parents`; every loop body and branch arm is reachable from its own tail. |
| 7 | Every expression parses under the bounded query grammar. |

Because the arena only appends and parents point strictly backward, and because
loop bodies and branch arms are nested rather than aliased into the outer
arena, no edge can point forward. A walk needs a visited set only to avoid revisiting shared
subgraphs, never to escape a cycle.

Error messages name the constraint, the offending value, and the expectation:

```text
Loop at index 5 (each-field) has max 0; max must be a positive integer literal
node 7 (propose-write) has parent 9 which is not less than its own index 7; edges must point backward
graph has 2 Terminal nodes at indices [11, 19]; exactly one is required
```

## Changing the schema

`IR.md` is the specification, not a description of the code. Adding a node kind
or a field means, in order: this file, the `ir/graph` JTD, the AssemblyScript
builder, the golden test bytes, then the Rust evaluator and its tests.
