# Dummy CRM form — end-to-end proof harness

Throwaway test scaffolding: a minimal contacts-list/search/edit web app served
by `server.js` (`node:http` only, no dependency), used to prove that a real
ThreadBox IR graph can drive real browser actions with an externally
observable outcome. This is **not** production code, not part of `guest/` or
`rust/`, and not subject to their zero-dependency rules.

## What it proves

`fixtures/crm-search-edit.ir.json` is a hand-authored `threadbox.ir.v1` graph
encoding: search "Jane" → click her row → click Edit → type a new address →
click Save → verify the confirmation banner → publish. It parses and passes
all four validators under `threadbox-ir` (see
`rust/threadbox-ir/tests/dummy_form_fixture.rs`).

`fixtures/locators.json` is a deterministic description→CSS-selector map,
standing in for a real vision model's `.locate()` resolution — the scope
decision documented in the implementation plan (option (a): hand-authored
fixture + locate stub, not a live `codex exec` scenario, for a CI-safe,
reproducible proof).

Walking the fixture's nodes in order and driving each `Click`/`Type` against
the real running server via Chrome DevTools Protocol produced exactly the
expected server-side action log:

```json
[
  {"action":"search","query":"Jane", ...},
  {"action":"click-row","id":1,"name":"Jane Doe", ...},
  {"action":"edit","id":1, ...},
  {"action":"save","id":1,"address":"123 New Street", ...}
]
```

and `GET /contact/1` afterward returned `"address":"123 New Street"`, with the
`#confirm-banner` element visible in the page — i.e. the `Verify` node's
assertion held. The IR's structure, not a hardcoded script, determined what
happened: this harness is intentionally not `tb-run` scope and does not claim
to "execute the graph" in the production sense — see `README.md`'s "Not in
scope here".

## Running it yourself

```sh
cd harness/dummy-form
./start.sh 3456
curl -s http://localhost:3456/contacts.json?q=jane
curl -s -X POST http://localhost:3456/reset   # clear the action log between runs
# then drive the browser (e.g. via an agent with CDP tools, or manually)
# against http://localhost:3456/, following fixtures/crm-search-edit.ir.json
# and fixtures/locators.json, and finally:
curl -s http://localhost:3456/log
./stop.sh
```

## Files

| File | Purpose |
|---|---|
| `server.js` | The dummy server: contacts list, search, contact detail/edit, action log. |
| `public/index.html` | Contacts list page. |
| `public/contact.html` | Contact detail/edit page. |
| `fixtures/crm-search-edit.ir.json` | The hand-authored IR fixture driving the proof. |
| `fixtures/locators.json` | Description → CSS selector map (the locate stub). |
| `start.sh` / `stop.sh` | Spawn/poll-ready/teardown for the server. |
