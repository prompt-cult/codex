# DSL.md — the language

The ThreadBox DSL is a library embedded in AssemblyScript. Its only purpose is to
build a graph in the guest's own linear memory. Compiling a program proves it is
well-formed at the type level; running the compiled module produces the graph and
calls `emit` once.

Nothing in this document performs I/O. There is no host round-trip during
construction, so no combinator can inspect a value.

Two properties follow from the vocabulary and matter more than the vocabulary:

- **No combinator can produce a back edge.** Repetition is only ever `Retry` with
  a literal bound or `ForEach` over a finite input. Termination is a structural
  question, decidable by walking the graph.
- **No combinator can branch on a live value.** `.branch()` consumes a `Verify`
  node, not a decision made during construction. The graph is a plan, not a
  partially evaluated execution.

## Scope

Repetitive data entry into a browser-based application with no programmatic
interface: take a structured submission, drive a form, verify each step, retry on
failure, escalate to a more capable model when retrying does not help, branch on
what is actually on screen, and iterate over the submission's fields.

Every construct below exists because that scenario needs it. Nothing else exists.

## Vocabulary

| Construct | Node | Meaning |
|---|---|---|
| `loadJson(name)` | `LoadJson` | A named input document, supplied from outside. The guest has no filesystem. |
| `screenshot()` | `Screenshot` | Capture the current view. |
| `.scale(width)` | `Scale` | Normalize to a fixed width with padding. Coordinates map back outside the graph. |
| `.locate(description, model?)` | `Locate` | Resolve a description of a control to viewport coordinates. |
| `.click()` | `Click` | Act at resolved coordinates. |
| `.type(value)` | `Type` | Enter a literal value at resolved coordinates. |
| `.typeField(ref)` | `Type` | Enter a value read from the input document. |
| `.confirm(assertion, model?)` | `Verify` | Second-opinion check yielding a boolean. |
| `.attempts(n)` | `Retry` | Bounded repetition. `n` is an integer literal. |
| `.orElse(alternative)` | `Fallback` | The escalation ladder. |
| `.branch(whenTrue, whenFalse)` | `Branch` | The only conditional. Consumes a `Verify`. |
| `Multi.over(ref).forEach(body)` | `ForEach` | Iteration bounded by an input array. `body` is a named top-level function. |
| `.then(next)` / `.after(prior)` | edge | Sequencing. Not a node. |
| `.publish()` | `Publish` | The single terminal. Exactly one per program. |

`.type(value)` and `.typeField(ref)` are two names for one node kind because the
AssemblyScript subset in use forbids function overloading. The node records which
form was used; see `IR.md`.

`.scale(width)` takes an explicit width, where the original sketch had `.scale()`
with none. The width is what makes a located coordinate meaningful: the graph
records the normalization it was reasoned about at, and the mapping back to full
page coordinates happens outside the graph. This document is normative and the
sketch is superseded.

`.locate()` and `.confirm()` both take an optional model. When it is omitted the
operator's configured default for that node kind applies; nothing about the graph
becomes unresolved.

`Multi.over(ref).forEach(body)` calls `body(element: FieldRef): Step`. The body
builds a **self-contained** chain — it starts with its own `screenshot()` and takes
no seed cursor — because the body is constructed before the `ForEach` node that
names it. That is what keeps every edge in the graph pointing backward; see
`IR.md`.

## Types

| Type | Produced by | Purpose |
|---|---|---|
| `Doc` | `loadJson(name)` | Handle to a named input document. |
| `FieldRef` | `doc.field(path)`, `doc.fields(path)` | An opaque reference to a value inside a document. Not a node. |
| `Step` | every combinator | A cursor into the arena. Chaining appends nodes. |

`FieldRef` is a string key paired with the document's arena index. It is not a
node kind: the node-kind count stays fixed at the twelve above. `field` names a
scalar; `fields` names an array and is what `Multi.over` accepts.

## Style rules the grader enforces

These are ordinary style rules, not boundary workarounds:

- Callbacks passed to `.branch()` and `.forEach()` are **named top-level
  functions**. A closure capturing enclosing scope is a failure.
- Every `.attempts(n)` bound is an integer literal, never a variable.
- There is exactly one `.publish()`. The guest asserts this before emitting.

## Worked example — grants data entry

The scenario: a beneficiary submission arrives as JSON. A legacy grants
case-management application must be driven to find or create the case, key every
field of the submission into it, save, and confirm the save happened.

```ts
import { loadJson, screenshot, Multi, Step, FieldRef, Doc } from "../assembly/ir";
import { emitGraph } from "../assembly/emit";
import { Spec, Tier, Providers, Model, Think, Context,
         withReviewMode } from "../assembly/models";

// The submission is declared, not read. The guest has no filesystem;
// whoever eventually runs the graph supplies the document by name.
const submission: Doc = loadJson("submission");

/// Find the search box and click it. The vision model is named
/// explicitly, so the record takes the catalog filter path and carries
/// no role: resolving pixels to coordinates is not one of the four
/// logical roles.
function openSearchBox(): Step {
  return screenshot()
    .scale(1280)
    .locate("the text input labelled 'Search' at the top of the page",
            Spec.model(Providers.Anthropic, Model.OpusPerformance)
                .think(Think.Medium)
                .context(Context.OneMillion))
    .click();
}

/// The case already exists: open it.
function openExistingCase(from: Step): Step {
  return from
    .then(screenshot().scale(1280))
    .locate("the first row of the search results table")
    .click();
}

/// The case does not exist: create it and key the identifying fields.
function createNewCase(from: Step): Step {
  return from
    .then(screenshot().scale(1280))
    .locate("the button labelled 'New Case'")
    .click()
    .then(screenshot().scale(1280))
    .locate("the input labelled 'Case Number'")
    .typeField(submission.field("caseNumber"))
    .then(screenshot().scale(1280))
    .locate("the textarea labelled 'Description'")
    .typeField(submission.field("description"));
}

/// One field of the submission. Each element of `answers` carries the
/// label to find and the value to key.
///
/// The body is self-contained: it starts its own chain and receives no
/// cursor, because it is built before the ForEach node that names it.
///
/// Three attempts against the default locator, then one escalation to a
/// stronger model. `attempts` is a literal, so the cost of this body is
/// bounded before the graph runs.
function enterField(element: FieldRef): Step {
  return screenshot()
    .scale(1280)
    .locate("the input whose visible label matches the element label")
    .click()
    .typeField(element)
    .attempts(3)
    .orElse(
      screenshot()
        .scale(1920)
        .locate("the input whose visible label matches the element label",
                Spec.model(Providers.Anthropic, Model.OpusPerformance)
                    .think(Think.High)
                    .context(Context.OneMillion))
        .click()
        .typeField(element)
    );
}

export function main(): void {
  const searched: Step = openSearchBox()
    .typeField(submission.field("caseNumber"))
    .then(screenshot().scale(1280));

  // The only conditional. It consumes a Verify node; it does not
  // observe a value during construction.
  const opened: Step = searched
    .confirm("the search results table contains at least one row",
             withReviewMode(Spec.tier(Tier.Balanced)))
    .branch(openExistingCase, createNewCase);

  const filled: Step = Multi.over(submission.fields("answers"))
    .forEach(enterField)
    .after(opened);

  filled
    .then(screenshot().scale(1280))
    .locate("the button labelled 'Save'")
    .click()
    .then(screenshot().scale(1280))
    .confirm("a confirmation banner reading 'Case saved' is visible",
             withReviewMode(Spec.tier(Tier.Balanced)))
    .attempts(2)
    .publish();

  emitGraph();
}
```

`emitGraph()` walks the arena, asserts there is exactly one `Publish`, serializes,
and calls the single import. It is the last statement of `main` and the only
effect the module has.

## Model selection

A separate builder in `guest/assembly/models.ts` producing inert records. It adds
no import and no export. Nothing in it opens a connection, reads an environment
variable, or knows a URL.

A record is built in two steps: `Spec` names the axes, and a role wrapper binds it
to a logical role. Keeping them separate is what avoids overloading — every
function below has exactly one signature, since the AssemblyScript subset in use
forbids two functions of the same name and forbids a parameter that accepts either
a tier or a vendor.

| Factory | Emitted record |
|---|---|
| `withPlanMode(Spec.tier(Tier.Performance))` | `{ role: "plan", tier: "performance" }` |
| `withReviewMode(Spec.tier(Tier.Balanced))` | `{ role: "review", tier: "balanced" }` |
| `Spec.model(Providers.Anthropic, Model.OpusPerformance).think(Think.Medium).context(Context.OneMillion)` | `{ vendor: "anthropic", model: "opus-performance", think: "medium", contextWindow: 1000000 }` |
| `Spec.on(Driver.OpenCodeGo, Model.QwenCode)` | `{ driver: "opencode-go", model: "qwen-code" }` |

`withBuildMode` and `withSummarizeMode` bind `role` to `code` and `summarize` the
same way. `.think()`, `.context()`, and `.on()` are chainable refinements that each
set one slot and return the record.

The record has seven slots: `role`, `tier`, `vendor`, `model`, `think`,
`contextWindow`, `driver`. **Absent slots are omitted from the serialized form and
impose no constraint** — a record constrains exactly the axes the author named.

There are exactly two resolution paths, and `tier` decides which:

- **`Spec.tier(...)` wrapped in a role** takes the policy path and defers entirely
  to the operator: whatever the operator currently calls the performance planner.
  Both `role` and `tier` are required, because the lookup key is the pair.
- **`Spec.model(...)` or `Spec.on(...)`** takes the catalog filter path, where zero
  matches and ambiguous matches are both hard configuration errors. These carry no
  `role`, because they name their model directly and no policy lookup happens. A
  vision model for `.locate()` is exactly this case.

Because the mapping lives outside the guest, retiring a model is a catalog edit and
never a change to any program.

Named constants exist so no program carries a magic number:

| Namespace | Members |
|---|---|
| `Tier` | `Eco`, `Balanced`, `Performance` |
| `Role` | `Plan`, `Code`, `Review`, `Summarize` |
| `Providers` | vendor of the weights — `Anthropic`, `OpenAI`, `Google`, `Moonshot`, `Alibaba`, `Mistral`, `Meta`, `Zhipu`, `DeepSeek`, `MiniMax`, `XAI` |
| `Driver` | who serves them — `OpenCodeZen`, `OpenCodeGo`, `Mistral`, `Groq`, `Ollama` |
| `Model` | catalog references, not wire identifiers |
| `Think` | `None`, `Low`, `Medium`, `High` |
| `Context` | window sizes present in at least one catalog entry |

`Model` constants are named for the catalog reference they emit, not for a notion
of recency: `Model.OpusPerformance` emits `"opus-performance"`. A constant whose
name and emitted string disagree would leave an implementer guessing which is
authoritative.

`vendor` and `driver` are separate axes: one driver serves several vendors, and
two drivers may both carry the same model.

Both `.locate()` and `.confirm()` may omit the model entirely, in which case the
operator's configured default for that node kind applies. When `.locate()` does
name one it uses the filter path, because vision is not one of the four roles;
`.confirm()` is a `review` judgement and takes `withReviewMode(Spec.tier(...))`
naturally.

## What is deliberately absent

- No loops other than `Retry` with a literal bound and `ForEach` over a finite
  input.
- No arithmetic, no string manipulation, no comparison operators on document
  values. A value is keyed in or referenced; it is never computed on.
- No way to name a URL, a header, a credential, or a file path.
- No escape hatch to call the host. The one import is called by `emitGraph()` and
  by nothing else.
