import { normalizeUsage, postJson } from "./http.mjs";

export async function callOpenAiChat(model, prompt, apiKey, fetchImpl) {
  // Reasoning models (e.g. Go kimi-k3) reject `temperature` outright with an
  // upstream HTTP 400, so it is opt-out per model via supportsTemperature.
  const body = {
    model: model.model,
    messages: [{ role: "user", content: prompt }],
    max_tokens: model.maxOutputTokens,
  };
  if (model.supportsTemperature !== false) {
    body.temperature = 0;
  }
  const data = await postJson(
    `${model.baseUrl}/chat/completions`,
    body,
    { authorization: `Bearer ${apiKey}` },
    fetchImpl,
  );
  const output = data.choices?.[0]?.message?.content;
  if (typeof output !== "string") {
    throw new Error("Chat Completions response did not contain text output");
  }
  return { output, tokenUsage: normalizeUsage(data.usage) };
}