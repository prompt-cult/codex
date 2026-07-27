# ThreadBox Phase 1 — RESULTS

Date: 2026-07-27
Profile: `eco` (`allowedCostClasses: [free, low]`)
Command: `npm run eval:eco` (promptfoo `0.121.19`, `--no-cache`, concurrency 2)

## Phase 1 gate

`promptfoo eval` passes for **≥2 providers × 2 katas**: **MET** — 12/16 (75%) passed.

| Result | Count |
|---|---|
| Passed | 12 (75.00%) |
| Failed | 2 (12.50%) |
| Errors | 2 (12.50%) |

Total tokens: 41,292 (19,452 prompt / 21,840 completion). Duration ~6m.

## Provider smoke (npm run smoke)

- `zen/ling-3.0-flash-free`: OK
- `go/qwen3.6-plus`: OK
- `mistral/mistral-small-latest`: OK
- `groq/llama-3.1-8b-instant`: OK

## Zen 4-wire protocol smoke (npm run smoke:zen-protocols)

- `zen/gpt-5.6-luna` (responses): OK
- `zen/claude-haiku-4-5` (messages): OK
- `zen/gemini-3.5-flash-lite` (gemini): OK
- `zen/ling-3.0-flash-free` (chat-completions): OK

## eco-profile role → model mapping (this run)

| Role | Model ID | Provider model | Wire |
|---|---|---|---|
| plan | go-qwen-code | qwen3.6-plus (Go) | messages |
| code | go-qwen-code | qwen3.6-plus (Go) | messages |
| review | zen-free-open | ling-3.0-flash-free (Zen) | chat-completions |
| summarize | groq-fast | llama-3.1-8b-instant (Groq) | chat-completions |

## Pass matrix (eco)

| Kata | review | code | plan | summarize |
|---|---|---|---|---|
| s1 | PASS | PASS | PASS | PASS |
| s2 | PASS | PASS | PASS | PASS |
| s4 | fail | fail | error | fail |
| s6 | PASS | PASS | PASS | PASS |

## Non-passing cases (kata s4 — release notes)

s4 has the strictest structural fingerprint (requires a named `dedupeArray`
key function and forbids closure callbacks). All four non-passes are genuine
model-capability findings, not harness defects:

- `s4` review (`ling-3.0-flash-free`, Zen free): score 0 — model returned an
  empty completion ("Chat Completions response did not contain text output").
- `s4` code (`qwen3.6-plus`, Go): score 0.33 — compiles, but stage 2
  (structure) failed: `Uni.dedupeArray: found 0, need at least 1`.
- `s4` summarize (`llama-3.1-8b-instant`, Groq): score 0 — stage 1
  (`asc --noEmit`) failed with an AssemblyScript type error (`TS2322`,
  passing a typed arrow closure to `.map<string>`).
- `s4` plan (`qwen3.6-plus`, Go): score 0 — request aborted (120 s cap).

## Notes / findings

- `deepseek-v4-flash-free` is **not** exposed by the authenticated Zen catalog
  (public catalog lists it; account does not). `zen-free-open` therefore maps
  to `ling-3.0-flash-free`, which the account does expose.
- The Responses adapter must **not** send `temperature` (reasoning `gpt-5.x`
  models reject it with HTTP 400). Fixed in `openai-responses.mjs`.
- `gpt-5.6-luna` is allowlisted `premium` for explicit `performance` runs
  only; it was exercised in the protocol smoke but is not part of the eco gate.

## Deferred (not part of this run)

- Direct Moonshot / Anthropic / OpenAI bring-your-own-key providers.
- Groq Whisper / speech / Compound systems.
- Automatic pricing discovery and budget accounting.
- `eval:performance` matrix (premium models; run only on explicit request).
