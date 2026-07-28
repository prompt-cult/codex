# Issue 3 ThreadBox Multimodel Provider Plan

## Goal

Build a minimal ThreadBox-owned provider layer for the Phase 1 DSL
evaluation harness. Prove OpenCode Zen, OpenCode Go, Mistral, and Groq
without extending Codex or its temporary Zen proxy.

## Changes

1. Add a curated model allowlist with `eco`, `balanced`, and
   `performance` profiles and logical `plan`, `code`, `review`, and
   `summarize` roles.
2. Add operator overrides for a profile, one role, or all roles.
3. Implement small adapters for OpenAI Responses, Anthropic Messages,
   OpenAI Chat Completions, and Gemini `generateContent`.
4. Wire four custom-provider instances into promptfoo.
5. Add mocked unit tests and opt-in capped live smoke tests.
6. Record model, provider, profile, wire protocol, and outcome in
   `RESULTS.md`.

## Safety and cost controls

- Only enabled model records may be selected.
- Overrides are validated before network I/O.
- API keys are read by name from the environment and never logged.
- Routine evaluation defaults to `eco`.
- Output tokens and concurrency are capped.
- Live smoke tests run sequentially and can be narrowed by provider.

## Verification

- Parse and validate both YAML configuration files.
- Run policy and adapter unit tests without network access.
- Run grader reference and negative self-checks.
- Verify configured model IDs against authenticated provider
  catalogues.
- Run one capped request for Zen, Go, Mistral, and Groq.
- Run capped Zen protocol-family smoke calls.
- Run the eco kata matrix and record all outcomes.
- `npm install` (runs `prepare` to install `../sdk`), `npm test`, and a
  real `promptfoo eval` in `eval-harness`; `just fmt` / `cargo test` /
  `just fix` from `codex-rs/` if Rust code changed.