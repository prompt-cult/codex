import { callAnthropicMessages } from "./anthropic-messages.mjs";
import { callGemini } from "./gemini.mjs";
import { callOpenAiChat } from "./openai-chat.mjs";
import { callOpenAiResponses } from "./openai-responses.mjs";
import { loadPolicy } from "./policy.mjs";
import { resolveSpec } from "./spec.mjs";

// Load the repo-root .env (gitignored) so provider API keys resolve when this
// provider runs under promptfoo, which does not source .env itself. Env vars
// already present in the real environment take precedence. Missing file is OK.
try {
  process.loadEnvFile(new URL("../../../.env", import.meta.url));
} catch {
  /* .env absent: rely on ambient process.env */
}

export default class ThreadBoxProvider {
  constructor(options = {}) {
    this.providerId = options.id || "threadbox";
    this.config = options.config || {};
  }

  id() {
    return this.providerId;
  }

  async callApi(prompt) {
    try {
      const { policyPath, catalogDir, ...spec } = this.config;
      const policy = await loadPolicy({ policyPath, catalogDir });
      const model = resolveSpec(policy, spec);
      const apiKey = resolveApiKey(model);
      const result = await callWire(model, prompt, apiKey);
      return {
        ...result,
        metadata: {
          driver: model.driver,
          model: model.model,
          catalogRef: model.id,
          catalogVersion: model.catalogVersion,
          tier: model.tier,
          role: model.role,
          wire: model.wire,
          costClass: model.costClass,
        },
      };
    } catch (error) {
      return { error: error instanceof Error ? error.message : String(error) };
    }
  }
}

/// Local drivers declare `requiresApiKey: false` and are called without
/// credentials; every remote driver must supply its declared env var.
export function resolveApiKey(model, environment = process.env) {
  if (!model.requiresApiKey) {
    return "";
  }
  const apiKey = environment[model.apiKeyEnv];
  if (!apiKey) {
    throw new Error(`${model.apiKeyEnv} is required for driver '${model.driver}'`);
  }
  return apiKey;
}

export function callWire(model, prompt, apiKey, fetchImpl = fetch) {
  switch (model.wire) {
    case "chat-completions":
      return callOpenAiChat(model, prompt, apiKey, fetchImpl);
    case "messages":
      return callAnthropicMessages(model, prompt, apiKey, fetchImpl);
    case "responses":
      return callOpenAiResponses(model, prompt, apiKey, fetchImpl);
    case "gemini":
      return callGemini(model, prompt, apiKey, fetchImpl);
    default:
      throw new Error(`unsupported wire protocol '${model.wire}'`);
  }
}