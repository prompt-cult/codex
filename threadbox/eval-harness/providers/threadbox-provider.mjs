import { callAnthropicMessages } from "./anthropic-messages.mjs";
import { callGemini } from "./gemini.mjs";
import { callOpenAiChat } from "./openai-chat.mjs";
import { callOpenAiResponses } from "./openai-responses.mjs";
import { loadPolicy, resolveModel } from "./policy.mjs";

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
      const policy = await loadPolicy(this.config.policyPath);
      const model = resolveModel(policy, this.config);
      const apiKey = process.env[model.apiKeyEnv];
      if (!apiKey) {
        return { error: `${model.apiKeyEnv} is required for ${model.provider}` };
      }
      const result = await callWire(model, prompt, apiKey);
      return {
        ...result,
        metadata: {
          provider: model.provider,
          model: model.model,
          modelId: model.id,
          profile: model.profile,
          wire: model.wire,
          costClass: model.costClass,
        },
      };
    } catch (error) {
      return { error: error instanceof Error ? error.message : String(error) };
    }
  }
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