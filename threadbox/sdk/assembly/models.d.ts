// ThreadBox AssemblyScript SDK -- model selection sub-DSL.
//
// This file is a *builder* only. Nothing here opens a connection,
// reads an environment variable, or knows a base URL. Every function
// below returns an inert `ModelSpec` record -- a flat bag of strings
// and one integer -- which the host resolves against the sysadmin
// policy table (`eval-harness/policy.yaml`) and the versioned driver
// catalogs (`eval-harness/catalogs/*.yaml`) via
// `eval-harness/providers/spec.mjs`.
//
// The split is deliberate: model identity drifts (models are retired
// unless they are premium), so the DSL commits only to *logical*
// intent and the operator owns the mapping. See PROVIDERS.md.
//
// Like `threadbox.d.ts`, this file is graded with `asc --noEmit`, so
// it stays inside the AssemblyScript subset: no function overloading
// (optional parameters instead), no string unions, no object literals.

/// Cost/capability tiers. A tier-only spec defers entirely to the
/// operator: `withPlanMode(Tier.Performance)` says "whatever the
/// sysadmin currently calls the performance planner".
export namespace Tier {
  export const Eco: string = "eco";
  export const Balanced: string = "balanced";
  export const Performance: string = "performance";
}

/// Model vendors, matched against a catalog entry's `vendor:` field.
/// This is not the driver axis: `opencode-zen` serves Anthropic,
/// OpenAI, Google, and Alibaba models through one endpoint, so vendor
/// and driver must stay separately addressable.
export namespace Providers {
  export const Anthropic: string = "anthropic";
  export const OpenAI: string = "openai";
  export const Google: string = "google";
  export const Moonshot: string = "moonshot";
  export const Alibaba: string = "alibaba";
  export const Mistral: string = "mistral";
  export const Meta: string = "meta";
  export const Zhipu: string = "zhipu";
  export const DeepSeek: string = "deepseek";
  export const MiniMax: string = "minimax";
  export const XAI: string = "xai";
  export const Nvidia: string = "nvidia";
  export const InclusionAI: string = "inclusionai";
}

/// Drivers -- the endpoint that serves a model. Only needed to
/// disambiguate a model that more than one driver carries (for example
/// `qwen-code`, served by both OpenCode Go and OpenCode Zen).
export namespace Driver {
  export const OpenCodeZen: string = "opencode-zen";
  export const OpenCodeGo: string = "opencode-go";
  export const Mistral: string = "mistral";
  export const Groq: string = "groq";
  export const Ollama: string = "ollama";
}

/// Model names. These are catalog *references*, not wire model ids:
/// `resolveSpec` accepts the catalog reference, the catalog key, or
/// the concrete wire id, so a vendor rename is a catalog edit and not
/// a kata edit.
export namespace Model {
  export const OpusLatest: string = "opus-performance";
  export const SonnetLatest: string = "sonnet-balanced";
  export const HaikuLatest: string = "haiku-eco";
  export const GptSol: string = "gpt-sol";
  export const GptTerra: string = "gpt-terra";
  export const GptLuna: string = "gpt-luna";
  export const GeminiFlash: string = "gemini-flash";
  export const GeminiFlashLite: string = "gemini-flash-lite";
  export const KimiCode: string = "kimi-code";
  export const KimiPerformance: string = "kimi-performance";
  export const QwenCode: string = "qwen-code";
  export const GlmCode: string = "glm-code";
  export const GemmaLocal: string = "gemma4";
}

/// Reasoning effort, matched against a catalog entry's `think:` field.
/// `None` is a real catalog value, not an absent constraint.
export namespace Think {
  export const None: string = "none";
  export const Low: string = "low";
  export const Medium: string = "medium";
  export const High: string = "high";
}

/// Context window sizes, matched exactly against a catalog entry's
/// `contextWindow:` field. Named constants exist so kata sources never
/// carry a magic number; every value below is present in at least one
/// catalog entry.
export namespace Context {
  export const OneHundredThirtyOneThousand: i32 = 131072;
  export const TwoHundredThousand: i32 = 200000;
  export const TwoHundredFiveThousand: i32 = 204800;
  export const TwoHundredFiftySixThousand: i32 = 256000;
  export const TwoHundredSixtyTwoThousand: i32 = 262144;
  export const FourHundredThousand: i32 = 400000;
  export const OneMillion: i32 = 1000000;
}

/// Roles a spec may bind to. `Build` is accepted as an alias of
/// `code` on the host side.
export namespace Role {
  export const Plan: string = "plan";
  export const Code: string = "code";
  export const Review: string = "review";
  export const Summarize: string = "summarize";
}

/// The record every builder call emits. Absent fields are the empty
/// string (or zero for `contextWindow`) and are dropped by
/// `normalizeSpec` before matching, so a spec constrains exactly the
/// axes the author named and nothing more.
///
/// Emitted shapes, mirroring the three documented call forms:
///
///   - `{ role: "plan", tier: "performance" }`
///   - `{ role: "plan", vendor: "anthropic", model: "opus-performance",
///        think: "high", contextWindow: 1000000 }`
///   - `{ role: "code", driver: "opencode-go", model: "qwen-code" }`
export class ModelSpec {
  role: string = "";
  tier: string = "";
  vendor: string = "";
  model: string = "";
  think: string = "";
  contextWindow: i32 = 0;
  driver: string = "";

  /// Renders the record as the JSON object the host reads. Only
  /// declared fields are emitted, so the wire form matches
  /// `normalizeSpec` exactly.
  toJSON(): string {
    let out = '{"role":"' + this.role + '"';
    if (this.tier.length > 0) out += ',"tier":"' + this.tier + '"';
    if (this.vendor.length > 0) out += ',"vendor":"' + this.vendor + '"';
    if (this.model.length > 0) out += ',"model":"' + this.model + '"';
    if (this.think.length > 0) out += ',"think":"' + this.think + '"';
    if (this.contextWindow > 0) out += ',"contextWindow":' + this.contextWindow.toString();
    if (this.driver.length > 0) out += ',"driver":"' + this.driver + '"';
    return out + "}";
  }
}

/// True when `value` names a driver rather than a vendor. Lets the
/// first positional argument carry either axis, which is what makes
/// `withBuildMode(Driver.OpenCodeGo, Model.QwenCode)` read naturally
/// while still disambiguating a model two drivers both carry.
function isDriver(value: string): bool {
  return (
    value == Driver.OpenCodeZen ||
    value == Driver.OpenCodeGo ||
    value == Driver.Mistral ||
    value == Driver.Groq ||
    value == Driver.Ollama
  );
}

/// True when `value` names a tier rather than a vendor. Keeps the
/// single-argument call form unambiguous without needing overloads.
function isTier(value: string): bool {
  return value == Tier.Eco || value == Tier.Balanced || value == Tier.Performance;
}

/// Shared constructor for all four role builders. A lone tier argument
/// takes the policy path; otherwise the first argument is routed to
/// `driver` when it names a driver and to `vendor` when it names a
/// vendor. `"mistral"` is both, and the Mistral driver serves only
/// Mistral models, so either reading selects the same candidates.
function buildSpec(
  role: string,
  tierOrVendorOrDriver: string,
  model: string,
  think: string,
  contextWindow: i32
): ModelSpec {
  const spec = new ModelSpec();
  spec.role = role;
  if (
    isTier(tierOrVendorOrDriver) &&
    model.length == 0 &&
    think.length == 0 &&
    contextWindow == 0
  ) {
    spec.tier = tierOrVendorOrDriver;
    return spec;
  }
  if (isDriver(tierOrVendorOrDriver)) {
    spec.driver = tierOrVendorOrDriver;
  } else {
    spec.vendor = tierOrVendorOrDriver;
  }
  spec.model = model;
  spec.think = think;
  spec.contextWindow = contextWindow;
  return spec;
}

/// Binds the planning role. Accepts either a tier
/// (`withPlanMode(Tier.Performance)`) or an explicit vendor or driver
/// plus any subset of model / think / context
/// (`withPlanMode(Providers.Anthropic, Model.OpusLatest, Think.High, Context.OneMillion)`).
/// Plain strings work identically -- the constants above are just
/// spelling aids and the host lower-cases every field.
export function withPlanMode(
  tierOrVendorOrDriver: string,
  model: string = "",
  think: string = "",
  contextWindow: i32 = 0
): ModelSpec {
  return buildSpec(Role.Plan, tierOrVendorOrDriver, model, think, contextWindow);
}

/// Binds the implementation role. `build` is the author-facing name;
/// the policy table keys it as `code`.
export function withBuildMode(
  tierOrVendorOrDriver: string,
  model: string = "",
  think: string = "",
  contextWindow: i32 = 0
): ModelSpec {
  return buildSpec(Role.Code, tierOrVendorOrDriver, model, think, contextWindow);
}

/// Binds the review role.
export function withReviewMode(
  tierOrVendorOrDriver: string,
  model: string = "",
  think: string = "",
  contextWindow: i32 = 0
): ModelSpec {
  return buildSpec(Role.Review, tierOrVendorOrDriver, model, think, contextWindow);
}

/// Binds the summarization role -- the cheap, high-volume,
/// domain-specific path that a small local model handles well.
export function withSummarizeMode(
  tierOrVendorOrDriver: string,
  model: string = "",
  think: string = "",
  contextWindow: i32 = 0
): ModelSpec {
  return buildSpec(Role.Summarize, tierOrVendorOrDriver, model, think, contextWindow);
}
