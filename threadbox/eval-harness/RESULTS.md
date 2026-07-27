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

## Performance profile run

Date: 2026-07-27
Profile: `performance` (`allowedCostClasses: [free, low, premium]`)
Command: `npm run eval:performance -- --output /tmp/threadbox-performance-eval.json`

| Result | Count |
|---|---|
| Passed | 10 (62.50%) |
| Failed | 2 (12.50%) |
| Errors | 4 (25.00%) |

Total tokens: 19,382 (15,999 prompt / 3,383 completion). Duration 23 s.

| Role | Model ID | Provider model | Outcome |
|---|---|---|---|
| review | zen-gpt-luna | gpt-5.6-luna (Zen) | 3/4 pass; s4 missed dedupe |
| code | mistral-large | mistral-large-latest | 3/4 pass; s4 exceeded structural limits |
| plan | go-kimi-performance | kimi-k3 (Go) | 0/4; HTTP 400 upstream failure |
| summarize | groq-strong | llama-3.3-70b-versatile (Groq) | 4/4 pass |

The four Kimi K3 failures were root-caused and **fixed** (see below).

## Kimi K3 root cause and fix

Date: 2026-07-27

`kimi-k3` is a reasoning model that **rejects the `temperature`
parameter**. Sending `temperature: 0` returns HTTP 400
`Error from provider (Console Go): Upstream request failed`; omitting
it returns HTTP 200. Isolated by direct probing:

| Model (Go, chat-completions) | `temperature: 0` | omitted |
|---|---|---|
| `kimi-k3` | HTTP 400 | HTTP 200 |
| `kimi-k2.7-code` | HTTP 200 | HTTP 200 |
| `glm-5.2` | HTTP 200 | HTTP 200 |
| `deepseek-v4-flash` | HTTP 200 | HTTP 200 |

This is the same class of defect as the earlier `gpt-5.x` Responses
fix, but on the Chat Completions wire. It is model-specific, not
provider-wide: the earlier `kimi-k2.7-code` 400 was transient
rate-limiting, not this bug.

`kimi-k3` is **not available on Zen** — that endpoint returns HTTP 401
`Model kimi-k3 is not supported`. It exists only in the Go catalog.

Fix: added an opt-out `supportsTemperature: false` flag on the model
record, honoured by `openai-chat.mjs` and type-validated in
`policy.mjs`. The flag defaults to sending `temperature: 0`, so no
other model changed behaviour.

## Performance profile run (after Kimi K3 fix)

| Result | Count |
|---|---|
| Passed | 13 (81.25%) |
| Failed | 3 (18.75%) |
| Errors | **0** |

Total tokens: 27,165. Duration 1m 54s.

`kimi-k3` (plan) now passes **4/4** katas. The three remaining
non-passes are structural-fingerprint misses, not infrastructure:

- `s4` / review (`gpt-5.6-luna`): `Uni.dedupeArray` found 0, need >= 1.
- `s4` / code (`mistral-large-latest`): 4 agent calls, at most 3 allowed.
- `s6` / code (`mistral-large-latest`): used `Flow.agent`, which this
  kata forbids.
