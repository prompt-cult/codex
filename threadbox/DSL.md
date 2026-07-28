# DSL.md — the authoring language

The DSL is a library embedded in AssemblyScript. Its only purpose is to build
an IR graph in the guest's own linear memory and serialize it once.

Nothing in this document performs I/O. There is no host round-trip during
construction, so no combinator can inspect a live value. A program that
compiles has proved its shape; it has not proved its behaviour.

Two properties hold by construction:

- **No back edges.** Repetition is `loop` with a literal bound and nothing
  else. There is no recursion. Termination is a structural question.
- **No branching on a live value.** A `guard` consumes an expression over
  values that already exist in the graph. The graph is a plan, not a partially
  evaluated execution.

## Vocabulary

| Constructor | Node | Meaning |
|---|---|---|
| `input(id, name, schemaUri)` | `Input` | Declare a named input document. |
| `tool(id, name, inputExpr, parents)` | `Tool` | Invoke one allowlisted tool. |
| `agent(id, binding, prompt, inputExpr, outputSchemaUri, parents)` | `Agent` | Invoke a configured model binding. |
| `transform(id, expr, parents)` | `Transform` | Apply a bounded JSON query. |
| `assertThat(id, expr, parent)` | `Guard` | Require a condition; pass the value through. |
| `validateWith(id, schemaUri, parent)` | `Guard` | Require the value to match a JTD. |
| `loop(id, varName, overExpr, max, parent, body)` | `Loop` | Bounded iteration over an array. |
| `branch(id, condExpr, parent, whenTrue, whenFalse)` | `Branch` | The only conditional. |
| `human(id, classification, inputExpr, parent)` | `Human` | Pause for a typed decision. |
| `checkpoint(id, label, inputExpr, parent)` | `Checkpoint` | Commit a durable replay boundary. |
| `terminal(id, status, outcome, parent)` | `Terminal` | End the run. Exactly one. |
| `emitGraph()` | — | Serialize and call the single import. |

Helpers `p1(a)`, `p2(a, b)`, `p3(a, b, c)` build the parent arrays, because
AssemblyScript has no variadic parameters and no array literals of class type
in the subset in use.

There is no `capture()`, no `click()`, no `type()`. Those are **tools**, not
language constructs. The language does not know what a screenshot is, and that
is deliberate: the vocabulary must not grow a term every time a tool is added.

## Expressions

Expression arguments are strings in the bounded query language. They evaluate
against `{ parents, vars, memo }` as described in `IR.md`.

```ts
".parents[0].document.driver.dateOfBirth"
".parents[0].files.items | map(select(.semanticKind == \"image\"))"
".vars.field.value"
".memo[\"observe-page\"].navigationId"
```

The grammar is deliberately small: path selection, indexing, iteration, pipe,
`map`, `select`, `length`, `type`, `has`, comparison, boolean operators,
literals, and object/array construction. There is no arithmetic beyond
comparison, no string manipulation, no function definition, and no way to reach
outside the value it was given.

### Absolute context references

A pipe rebinds `.` to the piped value, and `map` rebinds it to each element. So
inside a filter, `.vars` is no longer reachable — which is exactly where a
filter usually needs the loop variable. `$parents`, `$vars`, and `$memo` are
**absolute**: they always name the top-level keys of the node's context,
however deeply the input has been rebound.

```ts
// Wrong: inside map, `.vars` is looked up on each element.
".memo[\"load\"].document.answers | map(select(.step == .vars.step.index))"

// Right: the loop variable is reached absolutely.
"$memo[\"load\"].document.answers | map(select(.step == $vars.step.index))"
```

Only those three names exist. `$env`, `$ENV`, and anything else are parse
errors, because the context is the entire world an expression can see.

Note also that `|` binds looser than a comparison, so `.xs | length == 3` means
`.xs | (length == 3)`. When the right-hand side needs the outer context, group
explicitly: `(.xs | length) == $vars.step.count`.

## Style rules

- Loop bodies are **named top-level functions**. A closure capturing enclosing
  scope is a failure, not a preference: named functions are what make the body
  a self-contained arena.
- Every `loop` bound is an integer literal, never a variable.
- Every node `id` is a stable kebab-case string. Ids appear in logs, in
  `memo`, and in checkpoints; renaming one is a graph version change.
- Exactly one `terminal`.
- Documentation comments use `///`.

## Worked example

`guest/examples/acme-motor-quotes.ts` is the alpha graph in full. Its shape:

```
load submission
  -> validate against its JTD
  -> plan the steps from the submission itself
  -> loop over steps (max 3)
       observe the page, assert the expected step
       loop over the fields mapped to this step (max 20)
            resolve by accessible name
            branch:
              resolved   -> use the handle
              unresolved -> capture, standardize, select images,
                            assert non-empty, invoke the vision binding,
                            assert confidence, resolve the point on the page,
                            assert it hit a control
            propose the write, gate it, apply it
            verify against a fresh read, assert it matched
            checkpoint
       assert every field on this step was entered
       branch: more steps -> advance;  last step -> hold
  -> assert all three steps completed
  -> terminal: awaiting_human_final_submit
```

Every claim in that outline is a node, and every "assert" is a guard that stops
the run rather than a comment. The expensive arm — capture, standardize,
select, model — runs only for the controls a name lookup could not reach; on
this form that is two of seventeen, which is a property of the graph rather
than of a prompt.

`emitGraph()` walks the arena, asserts there is exactly one terminal, and calls
the single import. It is the last statement of `main` and the only effect the
module has.

## Deliberately absent

- No loop other than `loop` with a literal bound.
- No arithmetic, no string manipulation, no computation on document values. A
  value is read, compared, or keyed in; it is never calculated.
- No way to name a URL, a header, a credential, a file path, or a module.
- No escape hatch to the host. The one import is called by `emitGraph()`.
- No submit.
