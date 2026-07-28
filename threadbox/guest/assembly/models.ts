/// Model selection: inert records describing intent, never a concrete
/// wire identifier. Building a record opens no connection, reads no
/// environment variable, and knows no URL. Resolution happens entirely
/// outside the guest, against `harness/catalogs/<driver>.yaml` and
/// `harness/policy.yaml`.
///
/// See `DSL.md` — Model selection — for the two resolution paths.

/// Logical performance/cost tiers. The operator's policy maps a tier to
/// a concrete catalog entry per role.
export namespace Tier {
  export const Eco: string = "eco";
  export const Balanced: string = "balanced";
  export const Performance: string = "performance";
}

/// The four logical roles a policy-path record can bind to.
export namespace Role {
  export const Plan: string = "plan";
  export const Code: string = "code";
  export const Review: string = "review";
  export const Summarize: string = "summarize";
}

/// Vendor of the weights, not who serves them.
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
}

/// Who serves the model. One driver serves several vendors, and two
/// drivers may both carry the same model.
export namespace Driver {
  export const OpenCodeZen: string = "opencode-zen";
  export const OpenCodeGo: string = "opencode-go";
  export const Mistral: string = "mistral";
  export const Groq: string = "groq";
  export const Ollama: string = "ollama";
}

/// Catalog references, not wire identifiers. The emitted string is the
/// catalog key: `Model.OpusPerformance` emits `"opus-performance"`. A
/// constant whose name and emitted string disagreed would leave an
/// implementer guessing which is authoritative.
export namespace Model {
  export const OpusPerformance: string = "opus-performance";
  export const QwenCode: string = "qwen-code";
}

/// Reasoning effort.
export namespace Think {
  export const None: string = "none";
  export const Low: string = "low";
  export const Medium: string = "medium";
  export const High: string = "high";
}

/// Usable input token window sizes present in at least one catalog entry.
export namespace Context {
  export const OneMillion: i32 = 1000000;
}

/// The record a `Spec` builder produces. Has seven slots: `role`,
/// `tier`, `vendor`, `model`, a reasoning-effort slot, `contextWindow`,
/// `driver`. Absent slots are omitted from the serialized form and
/// impose no constraint — a record constrains exactly the axes the
/// author named.
///
/// The reasoning-effort slot cannot be named `think` on this class: a
/// field and the chainable `.think()` refinement below cannot share one
/// name in this language, so the field is `thinkLevel` and the emitted
/// JSON key (written by `emit.ts`) stays `think`.
export class ModelSpec {
  role: string | null;
  tier: string | null;
  vendor: string | null;
  model: string | null;
  thinkLevel: string | null;
  contextWindow: i32;
  driver: string | null;

  constructor() {
    this.role = null;
    this.tier = null;
    this.vendor = null;
    this.model = null;
    this.thinkLevel = null;
    this.contextWindow = 0;
    this.driver = null;
  }

  /// Chainable refinement: set reasoning effort. Returns `this`.
  think(value: string): ModelSpec {
    this.thinkLevel = value;
    return this;
  }

  /// Chainable refinement: set the usable context window in tokens.
  /// Returns `this`.
  context(tokens: i32): ModelSpec {
    this.contextWindow = tokens;
    return this;
  }

  /// Chainable refinement: bind this record to a driver directly.
  /// Returns `this`.
  on(driverName: string, modelRef: string): ModelSpec {
    this.driver = driverName;
    this.model = modelRef;
    return this;
  }
}

/// Two factories, one per resolution path. `Spec.tier(...)` is always
/// wrapped in a role wrapper below; `Spec.model(...)` and `Spec.on(...)`
/// name a model directly and carry no role.
export namespace Spec {
  /// Policy path: defers to the operator's table for `role` × `tier`.
  /// `role` and `tier` are both required, so this is always wrapped in
  /// `withPlanMode`, `withBuildMode`, `withReviewMode`, or
  /// `withSummarizeMode` before use.
  export function tier(value: string): ModelSpec {
    const spec = new ModelSpec();
    spec.tier = value;
    return spec;
  }

  /// Catalog filter path: names vendor and model directly. Carries no
  /// `role`, because no policy lookup happens.
  export function model(vendor: string, modelRef: string): ModelSpec {
    const spec = new ModelSpec();
    spec.vendor = vendor;
    spec.model = modelRef;
    return spec;
  }

  /// Catalog filter path: names driver and model directly.
  export function on(driverName: string, modelRef: string): ModelSpec {
    const spec = new ModelSpec();
    spec.driver = driverName;
    spec.model = modelRef;
    return spec;
  }
}

/// Bind a `Spec.tier(...)` record to the `plan` role.
export function withPlanMode(spec: ModelSpec): ModelSpec {
  spec.role = Role.Plan;
  return spec;
}

/// Bind a `Spec.tier(...)` record to the `code` role.
export function withBuildMode(spec: ModelSpec): ModelSpec {
  spec.role = Role.Code;
  return spec;
}

/// Bind a `Spec.tier(...)` record to the `review` role.
export function withReviewMode(spec: ModelSpec): ModelSpec {
  spec.role = Role.Review;
  return spec;
}

/// Bind a `Spec.tier(...)` record to the `summarize` role.
export function withSummarizeMode(spec: ModelSpec): ModelSpec {
  spec.role = Role.Summarize;
  return spec;
}
