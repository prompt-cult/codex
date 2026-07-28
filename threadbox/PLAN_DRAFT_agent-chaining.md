# PLAN_DRAFT — Bounded agent-to-agent computation in the DSL

> Draft pending GitHub issue creation. Rename to `PLAN_<issue-number>.md` once
> the issue below is filed, per `AGENTS.md`'s Markdown-Driven-Development
> order. This file's own Phase 1 (documentation) is already complete as of
> this commit; Phases 2–5 (code) have not started.

## Draft GitHub issue (what/why only — not yet filed)

**Title:** Bounded agent-to-agent computation in the DSL

**What:**
Introduce two new node kinds — `Prompt` and `Transform` — to the ThreadBox
IR, along with corresponding DSL combinators. `Prompt` represents a single,
bounded model invocation that receives a prompt (literal text or a named
template key resolved by the host) and produces either free text or
structured JSON. `Transform` represents a host-evaluated structural query
(jq/xq expression) that reshapes, filters, or expands a JSON value. Together
they enable pipelines of the form: *summarize → extract fields → filter
unresolved → escalate each to a stronger model → publish*.

**Why:**
The twelve existing node kinds describe vision-driven browser automation. A
growing class of workflows requires pre-computation before the first UI
interaction: summarizing a support ticket, extracting structured entities
from free text, triaging intent. These steps are bounded, non-recursive, and
produce no back edges — they fit the DAG contract exactly — but no existing
node kind can express them. Without first-class IR nodes, authors are forced
to pre-compute outside the graph (losing auditability) or embed raw strings
in `Type` nodes (losing type safety and validator coverage). Adding `Prompt`
and `Transform` keeps the graph as the single source of truth for the entire
plan, including its reasoning steps.

**Goals:**
- The graph remains a static DAG; no node inspects its own output during construction.
- Total model calls are finite and readable from the graph by counting `Prompt` nodes and their `Retry` bounds.
- jq/xq expressions are opaque strings in the IR; the host evaluates them; `threadbox-ir` adds zero new dependencies.
- The "no arithmetic, no string manipulation, no comparison operators on document values" rule is unchanged for everything the guest controls.

**Non-goals:**
- General arithmetic or string manipulation in the DSL (remains forbidden).
- Guest-side jq evaluation.
- Checkpoint/resume semantics in the IR (belongs to a future executor, not this repo).
- Unbounded fan-out (every `expand` requires a literal `maxItems`).

## Phase 1: Documentation (complete as of this commit)

- [x] README.md — new "Agent chaining and data reshaping" section.
- [x] AGENTS.md — denylist additions (no unbounded chains, no reactive-stream vocabulary), guest style additions (`.asText()`/`.asJSON()` mandatory finalizer, transform expressions are opaque strings, no memento node), Rust style addition (jq/xq is host-side, `threadbox-ir` stays zero-dependency).
- [x] DSL.md — vocabulary rows; "Prompt templates", "Agent call chaining", "Data reshaping", "Checkpoints", "`ForEach` over model output" subsections; a new worked example ("ticket triage"); "What is deliberately absent" addendum; kind-count updated to fourteen.
- [x] IR.md — `Prompt`/`Transform` node-kind rows; parent-order notes; `ForEach` over agent output subsection; `Prompt`/`Transform` field-shape catalog fragment; validators V5–V9 with exact failure-message shapes; structural invariants updated to fourteen kinds and the three new discriminators.

**Open questions surfaced by Phase 1, not yet resolved (see DSL.md's "ticket
triage" worked-example caveat and IR.md's field-shape-catalog note):**

1. Whether `FieldRef` can name an element of a `Prompt`/`Transform`'s own
   array output (needed for `step.forEach(body)` bodies to reference the
   current element the way `Multi.over(doc.fields(...)).forEach(...)` already
   does for document arrays) — genuinely open, must be resolved before Phase 2
   writes `Step.forEach`.
2. Whether `{slot}` placeholder syntax inside a literal prompt should be
   validated for non-empty names at parse time, or left entirely to the host.
3. Whether `select` (filter) should also carry an optional `maxItems` safety
   bound, or whether that is a follow-on issue.
4. Confirm `project`/`select`/`expand` are the desired verb choices before
   Phase 2 — renaming after `IR.md` ships as normative is a breaking change.

## Phase 2: Guest code (blocked on resolving the open questions above; not started)

### `guest/assembly/agents.ts` (new module, mirrors `models.ts`)
- [ ] `ResponseKind` namespace: `Text = "text"`, `Json = "json"`.
- [ ] `TransformOp` namespace: `Project = "project"`, `Select = "select"`, `Expand = "expand"`.
- [ ] `MAX_EXPAND_ITEMS: i32` named constant.
- [ ] Keep module under 500 LOC excluding tests.

### `guest/assembly/ir.ts`
- [ ] `Kind.Prompt`, `Kind.Transform` constants.
- [ ] `Node` fields: `promptKind`, `prompt`, `responseKind`, `op`, `expr`, `maxItems` (sentinel `-1` for absent).
- [ ] `CallBuilder` class with `.asText()` / `.asJSON()` finalizers (mirrors `ForEachBuilder`).
- [ ] `ask(prompt, source, model)`, `askName(key, source, model)` standalone functions.
- [ ] `Step.ask(prompt, model)`, `Step.askName(key, model)` → `CallBuilder`.
- [ ] `Step.project(expr)`, `Step.select(expr)`, `Step.expand(expr, maxItems)` → `Step`.
- [ ] `Step.forEach(body)` for non-document `ForEach` sources — named top-level function body, per existing style rule.
- [ ] If `ir.ts` approaches 450 LOC, split `CallBuilder` into `builders.ts` rather than growing the file.

### `guest/assembly/emit.ts`
- [ ] `Prompt` branch in `jsonNodeFields()`: emit `promptKind`, `prompt`, `responseKind`, `model`, in that order.
- [ ] `Transform` branch: emit `op`, `expr`, and `maxItems` only when `op == "expand"`.
- [ ] Update the unreachable-kind assert message to list all fourteen kinds.

## Phase 3: Guest tests (after Phase 2)

- [ ] `guest/tests/fixtures/prompt-pipeline.ts` — small graph exercising `LoadJson → Prompt(literal,json) → Transform(select) → ForEach(body: Prompt(name,text) → Retry) → Publish`.
- [ ] `guest/tests/fixtures/prompt-pipeline.json` — expected byte-exact output.
- [ ] Golden comparison test added to the existing `guest/tests/` runner.

## Phase 4: Rust reader (after Phase 3's golden fixture is committed — the Rust reader is regenerated from what the guest actually emits, never the reverse)

### `rust/threadbox-ir/src/lib.rs`
- [ ] `PromptKind` enum (`Literal`, `Name`) — mirrors `ValueKind`.
- [ ] `ResponseKind` enum (`Text`, `Json`).
- [ ] `TransformOp` enum (`Project`, `Select`, `Expand`).
- [ ] `NodeKind::Prompt { prompt_kind, prompt, response_kind, model }` variant.
- [ ] `NodeKind::Transform { op, expr, max_items: Option<u32> }` variant.
- [ ] Update `NodeKind::name()` for both new variants; kind-count comment to fourteen.

### `rust/threadbox-ir/src/parse.rs`
- [ ] `"Prompt"` arm: `promptKind` (required, enum), `prompt` (required string), `responseKind` (required, enum), `model` (required object — reuses existing `parse_model`).
- [ ] `"Transform"` arm: `op` (required, enum), `expr` (required non-empty string), `maxItems` (optional u32, present only when `op == "expand"`).
- [ ] Unknown-value rejection for `promptKind`/`responseKind`/`op`, same pattern as `Type`'s `valueKind`.
- [ ] Update unknown-kind error message to list fourteen kinds.
- [ ] Keep module under 500 LOC.

### `rust/threadbox-ir/src/validate.rs`
- [ ] `validate_prompt_model` (V5).
- [ ] `validate_transform_source` (V6) — source must be `LoadJson | Prompt | Transform`.
- [ ] `validate_foreach_prompt_source` (V7) — `ForEach` source `Prompt` must have `responseKind: "json"`.
- [ ] `validate_expand_max_items` (V8 + V9) — `expand` requires `maxItems`; `maxItems ≤ MAX_EXPAND_ITEMS`.
- [ ] Wire V5–V9 into `validate_graph()` after V4, in that exact order.

### `rust/threadbox-ir/src/error.rs`, `src/json.rs`
- [ ] No changes expected — existing `fail`/`fail_msg` helpers and the existing string/integer/object JSON shapes already cover every new field type.

## Phase 5: Rust tests (after Phase 4)

### `rust/threadbox-ir/tests/structural.rs`
- [ ] Parse: `Prompt` with `promptKind: "literal"`.
- [ ] Parse: `Prompt` with `promptKind: "name"`.
- [ ] Parse: `Transform` with each of `"project"` / `"select"` / `"expand"` (+ `maxItems`).
- [ ] Reject: unknown `promptKind` / `responseKind` / `op` value.
- [ ] Reject: `Prompt` missing `model`.

### `rust/threadbox-ir/tests/validators.rs`
- [ ] V5: `Prompt` without `model` rejected, exact message.
- [ ] V6: `Transform` with a `Click` source rejected, exact message.
- [ ] V7: `ForEach` with a text-`responseKind` `Prompt` source rejected, exact message.
- [ ] V8: `expand` without `maxItems` rejected, exact message.
- [ ] V9: `expand` with `maxItems` over the configured maximum rejected, exact message.
- [ ] Ordering: a graph failing both V3 (bad `Retry` bound) and V5 (missing `model`) must report V3 first.
- [ ] Full pipeline: `LoadJson → Prompt(json) → Transform(select) → ForEach → Publish` passes all nine validators.

## Phase 6: Re-verify documentation matches reality (after Phase 5 is green)

- [ ] Re-read README.md, AGENTS.md, DSL.md, IR.md against the finished code.
- [ ] Confirm no existing golden fixture's byte output changed.
- [ ] Confirm every place asserting the "unknown kind" error text was updated for fourteen kinds.
- [ ] Resolve and document the four open questions above, updating DSL.md's caveats accordingly.
