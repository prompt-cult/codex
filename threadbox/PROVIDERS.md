# ThreadBox Providers

ThreadBox uses logical roles and operator-owned tiers instead of
embedding concrete model IDs in DSL programs. The Phase 1 provider
proof supports OpenCode Zen, OpenCode Go, Mistral, Groq, and a local
Ollama runtime.

## Two ownership boundaries

Model configuration is split into two files with different owners and
different change cadences:

| File | Owner | Answers |
|---|---|---|
| `eval-harness/catalogs/<driver>.yaml` | driver author | what does this vendor currently serve? |
| `eval-harness/policy.yaml` | sysadmin | which logical constant maps to which catalog entry? |

A driver catalog is **versioned** (`catalogVersion`) and replaced
wholesale when a vendor adds or retires models. `policy.yaml` is edited
by whoever operates the deployment, and environment variables override
it. Neither file is reachable from generated AssemblyScript.

## Tier and role lexicon

A DSL program never names a model. It names a **tier** and a **role**,
written `Tier::Role`:

| Tier | Meaning | Cost classes admitted |
|---|---|---|
| `Eco` | cheapest thing that can do the job; local or free where possible | `free`, `low` |
| `Balanced` | good enough for daily work, below the vendor price spike | `free`, `low`, `standard` |
| `Performance` | best available, long context, reasoning enabled | `free`, `low`, `standard`, `premium` |

| Role | Alias | Meaning |
|---|---|---|
| `plan` | — | decompose a task, choose an approach |
| `code` | `build` | emit or edit the DSL / source |
| `review` | — | judge an artefact against a contract |
| `summarize` | — | compress transcript or output |

`Balanced::Plan`, `Balanced::Build`, `Performance::Plan` and
`Performance::Build` are therefore the four cells an agent normally
addresses. Tier names are matched case-insensitively, so
`THREADBOX_TIER=Performance` and `THREADBOX_TIER=performance` are the
same tier. `THREADBOX_PROFILE` is accepted as an alias of
`THREADBOX_TIER`.

Cost classes are ordered `free < low < standard < premium`. `standard`
is the band `Balanced` needs: stronger than the free open models, still
below the vendor price spike that `premium` sits above.

## Capability metadata

Model identifiers are not durable: non-premium models are retired on a
timescale of months, so a tier table pinned to concrete IDs rots. Each
catalog entry therefore carries vendor-neutral capability metadata that
a builder specification can select against:

| Field | Meaning |
|---|---|
| `vendor` | who trained the weights (distinct from the driver, who serves them) |
| `contextWindow` | usable input tokens |
| `think` | native reasoning effort: `none`, `low`, `medium`, or `high` |
| `costClass` | `free`, `low`, `standard`, or `premium` |
| `roles` | logical roles this entry is permitted to serve |

## Driver catalog schema

```yaml
driver: opencode-zen
catalogVersion: 2026-07-27
baseUrl: https://opencode.ai/zen/v1
apiKeyEnv: OPENCODE_API_KEY
models:
  opus-performance:
    model: claude-opus-5
    wire: messages
    vendor: anthropic
    contextWindow: 1000000
    think: high
    costClass: premium
    roles: [plan, code, review]
    maxOutputTokens: 4096
```

Catalog entries are addressed globally as `driverId:modelId`, for
example `opencode-zen:opus-performance`. `catalogVersion` is mandatory;
a duplicate or unknown `driverId:modelId` is a load-time error.

`ollama.yaml` sets `requiresApiKey: false` and
`baseUrl: http://localhost:11434/v1`. Ollama is the free local driver
for narrow domain-specific work such as summarization; when the daemon
is not running the live smoke reports `SKIP` rather than failing.

## Sysadmin policy schema

```yaml
version: 1
defaultTier: eco
tiers:
  eco:
    allowedCostClasses: [free, low]
    plan:  opencode-go:qwen-code
    code:  opencode-go:qwen-code
    review: opencode-go:kimi-code
    summarize: ollama:gemma4
```

Resolution precedence:

1. `THREADBOX_FORCE_MODEL` forces one enabled, allowlisted entry for
   every compatible role.
2. `THREADBOX_ROLE_<ROLE>` overrides one logical role.
3. `THREADBOX_TIER` (alias `THREADBOX_PROFILE`) selects a tier.
4. `defaultTier` in `policy.yaml` is used otherwise.

The cost-class gate is applied after every override, so a `premium`
entry is still rejected under `eco` or `balanced` even when forced by
environment variable.

## The builder sub-DSL

A DSL program declares intent through a builder that produces a plain
specification record. The builder performs no I/O: it emits key/value
data that a provider factory resolves later.

| Form | Emitted spec |
|---|---|
| `withPlanMode(Tier.Performance)` | `{ role: "plan", tier: "performance" }` |
| `withPlanMode(Providers.Anthropic, Model.OpusLatest, Think.Medium, Context.OneMillion)` | `{ role: "plan", vendor: "anthropic", model: "opus-latest", think: "medium", contextWindow: 1000000 }` |
| `withPlanMode("Anthropic", "Opus5", "medium", 1000000)` | same shape, strings normalized lower-case |

`withBuildMode`, `withReviewMode`, and `withSummarizeMode` are the same
overloads with `role` set to `code`, `review`, and `summarize`.

`resolveSpec()` takes the tier path when only a tier is present.
Otherwise it filters the merged allowlist on the declared fields. Zero
matches and ambiguous matches are both hard configuration errors that
name the specification and the rejected candidates.

## Supported protocols

| Wire protocol | Initial drivers and models |
|---|---|
| OpenAI Responses | Zen GPT models |
| Anthropic Messages | Zen Claude models; Go Qwen and MiniMax models |
| OpenAI Chat Completions | Zen and Go open models; Mistral; Groq; Ollama |
| Gemini `generateContent` | Zen Gemini models |

Provider responses are normalized to text, token usage, and non-secret
metadata. Protocol adapters do not decide which model is permitted.

`supportsTemperature: false` exists because reasoning models reject the
field: `kimi-k3` on OpenCode Go returns HTTP 400 when `temperature` is
sent.

## Initial proof

The routine matrix uses one economical representative per driver:

- Zen: a current free open model.
- Go: Qwen3.6 Plus.
- Mistral: Mistral Small.
- Groq: Llama 3.1 8B Instant.
- Ollama: Gemma 4 26B MoE, local and free.

Separate capped smoke calls prove Zen's Responses, Messages, Gemini,
and Chat Completions endpoints. Premium models are allowlisted only for
explicit `performance` runs.

## Deferred providers and capabilities

- Direct Moonshot with `MOONSHOT_API_KEY`.
- Direct Anthropic and OpenAI bring-your-own-key providers.
- Groq Whisper, speech, and Compound systems.
- Automatic pricing discovery, budget accounting, and model
  benchmarking.
- Dynamic model selection by generated code.

The configuration schema reserves space for these additions without
creating incomplete adapters.
