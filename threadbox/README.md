# ThreadBox

ThreadBox is an agent-DSL for expressing composable, statically-shaped
work graphs (fork/join, parallel review, bounded loops, scheduled
digests) in AssemblyScript, compiled to Wasm and executed inside a
`wasmi` host that calls back out to sandboxed `codex exec` agents and
capability-registry-only I/O.

This folder is **Phase 1 only**: a `promptfoo`-driven kata harness that
answers one question before any runtime is built — *can candidate
models actually write this DSL correctly?* There is no Rust code here
and no Wasm execution; grading is done by type-checking (`asc
--noEmit`), structural fingerprinting, and static lint.

Phase 1 owns its provider policy rather than routing through Codex.
OpenCode Zen, OpenCode Go, Mistral, Groq, and a local Ollama runtime are
described by versioned driver catalogs in `eval-harness/catalogs/`, and
mapped to logical constants by a single sysadmin-owned
`eval-harness/policy.yaml`. DSL programs identify a tier and a logical
role such as `plan`, `code`, `review`, or `summarize`; they never choose
unrestricted model IDs or provider URLs.

See the top-level plan (`submit_plan` output from the ThreadBox
planning session) for the full 14-phase roadmap. This README only
covers what exists in this folder today.

## Layout

```
threadbox/
  sdk/
    assembly/threadbox.d.ts   # hand-written AS type declarations (primitives-only ABI)
    assembly/ABI.md           # @external module/function names + param order (canonical reference)
    package.json              # pins assemblyscript version used for `asc --noEmit`
  eval-harness/
    package.json              # pins promptfoo (separate from sdk's package.json on purpose)
    catalogs/                 # versioned per-driver model catalogs
    policy.yaml               # sysadmin-owned tier x role mapping
    promptfooconfig.yaml       # providers x prompts x graders matrix
    providers/                 # policy/spec resolution and wire-protocol adapters
    prompts/
      system.md                # SDK docs + 2 worked reference examples, injected into every kata
      kata-s1.md               # parallel review of the current change (fork/join/rank)
      kata-s2.md                # diagnose a failing test (independent investigators join)
      kata-s4.md                # release notes (2-source, 2-agent fan-out then join+dedupe)
      kata-s6.md                # service health (3-way endpoint() fanout)
    graders/
      grade.mjs                 # extract code -> asc --noEmit -> fingerprint -> lint (3-stage gate)
      check-structure.mjs       # structural fingerprint matcher used by grade.mjs
    fingerprints/
      s1.json s2.json s4.json s6.json   # required-call-set + node/join counts per kata
    examples/
      parallel-review.ts        # worked reference solution for kata-s1 (used in system.md)
      health-check.ts           # worked reference solution for kata-s6 (used in system.md)
    RESULTS.md                  # populated after `promptfoo eval` runs
```

## Why two `package.json` files

`sdk/package.json` pins the `assemblyscript` compiler version used only
for `asc --noEmit` type-checking of generated solutions. `eval-harness/package.json`
pins `promptfoo` and any lint deps. Keeping them separate prevents
toolchain version drift between "does this parse as valid AS" and
"how do we run the eval matrix" — the two concerns evolve on different
schedules (AS compiler bumps vs promptfoo config changes).

## What "green" means for Phase 1

`promptfoo eval` using the `eco` tier passes for at least 2
providers x 2 katas. The full Zen, Go, Mistral, and Groq x 4-kata
matrix is a stretch goal tracked in `RESULTS.md`, not a blocking gate.

## Model policy

Model configuration is split by ownership. A **driver catalog** under
`catalogs/` states what a vendor currently serves and carries a
`catalogVersion`; it is replaced wholesale when models are added or
retired. `policy.yaml` states which logical constant maps to which
catalog entry and is edited by the sysadmin.

Programs address a tier and a role, written `Tier::Role`:

- `eco` uses free, subscription, local, or economical models for
  continuous regression.
- `balanced` uses stronger models below the vendor price spike.
- `performance` permits premium models with long context and reasoning
  enabled.

Roles are `plan`, `code` (alias `build`), `review`, and `summarize`.

Resolution precedence is:

1. `THREADBOX_FORCE_MODEL` forces one enabled, catalogued entry for
   every compatible role.
2. `THREADBOX_ROLE_<ROLE>` overrides one logical role.
3. `THREADBOX_TIER` (alias `THREADBOX_PROFILE`) selects a tier.
4. `defaultTier` in `policy.yaml` is used otherwise.

Unknown, disabled, incompatible, or over-budget selections fail before
any network call, including when forced by environment variable.
Generated AssemblyScript cannot change this policy. See `PROVIDERS.md`
for the catalog and policy schemas and the builder sub-DSL.

## Provider keys

Provider secrets are read only from environment variables:

- `OPENCODE_API_KEY` for both OpenCode Zen and OpenCode Go.
- `MISTRAL_API_KEY` for Mistral.
- `GROQ_API_KEY` for Groq.
- Ollama needs no key; it is reached at `http://localhost:11434/v1` and
  smoke-tested as `SKIP` when the daemon is not running.

The provider layer never prints keys or authorization headers. The
local `.env` file is for development only and must remain ignored.
Direct Moonshot, Anthropic, and OpenAI providers are deferred.

## Non-goals for this folder

- No Wasm compilation (`asc --noEmit` only, never full `asc` codegen).
- No wasmi runtime, no callback dispatch, no sandboxed `asc` execution.
- No Codex or zen-proxy changes. Phase 1 calls provider endpoints
  through ThreadBox's own small adapters.
- No dynamic graph mutation, no persistence, no daemon. Those are
  later phases against `codex-threadbox-core` (a genuine Rust crate,
  not yet created).
