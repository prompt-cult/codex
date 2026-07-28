# Dummy CRM form — the end-to-end demo

Throwaway test scaffolding: a minimal contacts-list/search/edit web app served
by `server.js`, used to answer the only question that matters about an IR — does
it mean anything operationally, or does it merely type-check and validate?

This directory is **not** production code, not part of `guest/` or `rust/`, and
not subject to their zero-dependency rules. `server.js` uses `node:http` and
nothing else, but that is convenience rather than obligation. Nothing here is
`tb-run` scope, and nothing here claims to "execute the graph" in the
production sense the root `README.md` defers to later — see "Not in scope
here" there.

## The app under test

A deliberately dull legacy-CRM impression: five contacts, a search box, a
contact detail page with an Edit button, an address field, and a Save button
that shows a confirmation banner. Every interaction the page performs is POSTed
to the server, which appends it to an in-memory action log. **That log is the
oracle** — it is how a replay proves what actually happened, rather than what a
transcript claims happened.

| Endpoint | Method | Purpose |
|---|---|---|
| `/` | GET | Contacts list page (`public/index.html`). |
| `/contact.html?id=<n>` | GET | Contact detail/edit page (`public/contact.html`). |
| `/contacts.json?q=<query>` | GET | Contact search, case-insensitive substring. |
| `/contact/<id>` | GET | One contact as JSON. |
| `/contact/save` | POST | Persist an address change. |
| `/action` | POST | The page reports one interaction; appended to the action log. |
| `/log` | GET | The action log so far. **The oracle.** |
| `/reset` | POST | Clear the action log and restore the original contacts. |
| `/health` | GET | Readiness probe used by `start.sh`. |
| `/holo.html` | GET | The vision round-trip page (second demo below). |
| `/guest/assembly/holo.wasm` | GET | Serves the guest's compiled coordinate algebra to that page. |
| `/holo/chat` | POST | Same-origin proxy to the HAI API, so the browser never sees the key. |
| `/trace/session`, `/trace`, `/trace/image` | POST | Per-session trace log and sent-image capture. |

```sh
cd harness/dummy-form
./start.sh 3456        # spawns, polls /health, writes .server.pid
./stop.sh              # tears down
```

## Demo 1 — deterministic IR replay

**Question:** can a real `threadbox.ir.v1` graph drive real browser actions to
an externally observable outcome?

`fixtures/crm-search-edit.ir.json` is a hand-authored 24-node graph:

```
Screenshot → Scale 1280 → Locate "the search input"      → Click → Type "Jane"
Screenshot → Scale 1280 → Locate "the row for Jane Doe"  → Click
Screenshot → Scale 1280 → Locate "the Edit button"       → Click
Screenshot → Scale 1280 → Locate "the address input field" → Click → Type "123 New Street"
Screenshot → Scale 1280 → Locate "the Save button"       → Click
Verify "Contact saved banner is visible" → Publish
```

It is a real graph, not a script: it parses and passes all four `IR.md`
validators under `threadbox-ir`, asserted by
`rust/threadbox-ir/tests/dummy_form_fixture.rs`, exactly like any graph the
guest emits.

`fixtures/locators.json` maps each `Locate` description to a CSS selector,
standing in for what a vision model would resolve. That substitution is the
deliberate scope decision: it keeps the proof deterministic and repeatable, so a
failure means "the replay is broken", never "the model had an off day". Demo 2
below removes the stub.

**Verification method.** Walk the fixture's nodes in index order; for each
`Locate` resolve the selector, for each `Click`/`Type` dispatch real Chrome
DevTools Protocol input against the running page; for `Verify` read the DOM;
then read `/log` and compare.

**Result.** The action log was exactly the prescribed sequence, in order:

```json
[
  {"action":"search",    "query":"Jane"},
  {"action":"click-row", "id":1, "name":"Jane Doe"},
  {"action":"edit",      "id":1},
  {"action":"save",      "id":1, "address":"123 New Street"}
]
```

`GET /contact/1` then returned `"address":"123 New Street"`, and
`#confirm-banner` was visible with `hidden === false`, satisfying the `Verify`
node's assertion before `Publish` terminated the graph. The graph's structure
determined what happened.

**One real finding.** A literal `Click` then `Type` against a pre-filled input
*appends*; it does not replace. Reproducing the intended edit needed a
select-all (triple-click) before typing. The `Type` node as specified in `IR.md`
carries no clearing semantics, so "what does `Type` mean against a field that
already has content" is an open question for whoever builds the real executor.
It is recorded here rather than papered over.

```sh
./start.sh 3456
curl -s -X POST http://localhost:3456/reset      # always reset between runs
# drive the browser against http://localhost:3456/ following
# fixtures/crm-search-edit.ir.json and fixtures/locators.json
curl -s http://localhost:3456/log                # the oracle
./stop.sh
```

## Demo 2 — live vision round-trip (Holo)

**Question:** with the locate stub removed, can a real vision model resolve a
`Locate` description to a coordinate, and can that coordinate be mapped back
into the page's own pixel frame *deterministically*?

The split matters. The model does perception; **the arithmetic that turns its
answer into a clickable pixel is compiled WebAssembly with unit tests**, not
JavaScript scattered through a page. `guest/assembly/holo.ts` holds that algebra
and nothing else — no `fetch`, no Canvas, no DOM:

| Export | Meaning |
|---|---|
| `normToSentPixel(norm, dimension)` | Holo's `[0,1000]` coordinate → pixel on the canvas that was sent. |
| `planScale(origW, origH, targetW, targetH)` | Aspect-preserving scaled content size inside the target canvas. Returns packed `(w,h)` as one `i64`. |
| `normToOriginalPixel(...)` | The full inverse: normalized coordinate + how the host scaled and padded → pixel in the original capture. Returns `-1` when the coordinate lands in the pad. |
| `unpackX` / `unpackY` | Unpack a packed `(x,y)` `i64` (no classes cross the WASM boundary). |

The load-bearing property, re-proven live: **Holo normalizes to the image it was
sent**, not to any canonical resolution. So scaling before sending is safe
provided the inverse undoes exactly that scale and that pad. `holo.ts` is what
guarantees the "provided".

`public/holo.html` is the host: it captures the viewport, scales and pads to a
fixed canvas, calls the model, and asks the WASM module for the original-frame
pixel. The API key never reaches the browser — the page POSTs same-origin to
`/holo/chat` and the server attaches the bearer token, which also sidesteps the
CORS preflight a direct page→API call would hit.

**Tracing.** Each page load allocates a time-prefixed session id. Every traffic
line is POSTed to `/trace` and every sent image to `/trace/image`, so a run
leaves a durable `logs/<session>/trace.jsonl` plus the exact PNG bytes the model
saw. That is deliberate: when a localize call lands wrong, the sent image and
the prompt are the evidence, and prompt tuning without them is guesswork.
`logs/` is gitignored.

**Result of one live run** (`logs/20260728021624-.../trace.jsonl`, verbatim):

```
captured viewport 776x506
WASM planScale → content 1280x835 in 1280x1600 canvas (pad 0,765)
POST /holo/chat  model=holo3-1-35b-a3b  image=330015B
                 target="the search input box at the top of the page"
← 200 (3965ms)  norm={"x":360,"y":100}  tokens={prompt:2104, completion:14}
WASM normToOriginalPixel → (279, 97) in original 776x506 viewport
```

So the full chain ran: real capture → WASM-planned scale and pad → real model
call → normalized answer → WASM inverse map → a concrete pixel in the page's own
frame. **Whether that pixel sits on the intended element is a separate
judgement**, and is exactly what the per-session PNG plus prompt text in
`logs/` exist to adjudicate. Treat the number above as evidence that the
round-trip is wired and deterministic, not as a claim about localisation
accuracy — that claim needs a rect comparison per target, which is the next
piece of work.

The coordinate algebra itself is not taken on trust. Five tests in
`guest/tests/holo.test.js` cover the endpoint mapping, a no-scale/no-pad
inversion against the live numbers above, a scale-to-1280-with-pad round trip
(within 2px), the pad sentinel returning `-1`, and aspect-ratio preservation.

```sh
# 1. build the coordinate algebra (holo.wasm is a gitignored artifact)
cd guest && npx asc assembly/holo.ts -o assembly/holo.wasm --exportRuntime

# 2. the key must be exported for the server; the page never sees it
set -a; . /Users/Shared/codex/.env; set +a     # provides HAI_API_KEY

# 3. serve and open the round-trip page
cd ../harness/dummy-form && ./start.sh 3456
open http://localhost:3456/holo.html

# 4. after a run, read the durable evidence
cat logs/<session>/trace.jsonl
```

## Verifying both demos' supporting tests

```sh
cd guest  && npm test                                   # 6 tests: 1 emit golden + 5 holo algebra
cd rust   && cargo test -p threadbox-ir                 # includes the IR fixture validity test
```

## Files

| File | Purpose |
|---|---|
| `server.js` | Dummy CRM, action log, HAI proxy, per-session tracing. |
| `public/index.html` | Contacts list page. |
| `public/contact.html` | Contact detail/edit page. |
| `public/holo.html` | Vision round-trip host page (capture, scale, call, inverse-map). |
| `fixtures/crm-search-edit.ir.json` | The 24-node IR graph replayed in demo 1. |
| `fixtures/locators.json` | Description → CSS selector map (demo 1's locate stub). |
| `start.sh` / `stop.sh` | Spawn/poll-ready/teardown. |
| `logs/` | Per-session `trace.jsonl` + sent PNGs. Gitignored. |
| `../../guest/assembly/holo.ts` | The coordinate algebra, compiled to WASM. |
| `../../guest/tests/holo.test.js` | Its five unit tests. |
| `../../rust/threadbox-ir/tests/dummy_form_fixture.rs` | Asserts demo 1's fixture is a valid graph. |
