# ThreadBox Providers

ThreadBox uses logical roles and operator-owned profiles instead of
embedding concrete model IDs in DSL programs. The Phase 1 provider
proof supports OpenCode Zen, OpenCode Go, Mistral, and Groq.

## Supported protocols

| Wire protocol | Initial providers and models |
|---|---|
| OpenAI Responses | Zen GPT models |
| Anthropic Messages | Zen Claude and Qwen models; Go Qwen and MiniMax models |
| OpenAI Chat Completions | Zen and Go open models; Mistral; Groq |
| Gemini `generateContent` | Zen Gemini models |

Provider responses are normalized to text, token usage, and
non-secret metadata. Protocol adapters do not decide which model is
permitted.

## Initial proof

The routine four-provider matrix uses one economical representative
from each provider:

- Zen: a current free open model.
- Go: Qwen3.6 Plus.
- Mistral: Mistral Small.
- Groq: Llama 3.1 8B Instant.

Separate capped smoke calls prove Zen's Responses, Messages, Gemini,
and Chat Completions endpoints. Premium models are allowlisted only
for explicit `performance` runs.

## Deferred providers and capabilities

- Direct Moonshot with `MOONSHOT_API_KEY`.
- Direct Anthropic and OpenAI bring-your-own-key providers.
- Groq Whisper, speech, and Compound systems.
- Automatic pricing discovery, budget accounting, and model
  benchmarking.
- Dynamic model selection by generated code.

The configuration schema reserves space for these additions without
creating incomplete adapters.