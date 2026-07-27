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
  // Local drivers such as Ollama serve the OpenAI wire without auth; sending
  // an empty bearer token there is worse than sending no header at all.
  const headers = apiKey ? { authorization: `Bearer ${apiKey}` } : {};
  const data = await postJson(
    `${model.baseUrl}/chat/completions`,
    body,
    headers,
    fetchImpl,
  );
  const choice = data.choices?.[0];
  const output = choice?.message?.content;
  if (typeof output !== "string") {
    throw new Error("Chat Completions response did not contain text output");
  }
  // Reasoning models spend the output budget on hidden reasoning first, so a
  // budget that is too small returns an empty `content` with finish_reason
  // "length" rather than an HTTP error.
  if (output === "" && choice.finish_reason === "length") {
    throw new Error(
      `${model.driver}/${model.model} truncated before emitting text: max_tokens=${model.maxOutputTokens} ` +
        `was consumed by reasoning (think=${model.think}, completion_tokens=${data.usage?.completion_tokens})`,
    );
  }
  return { output, tokenUsage: normalizeUsage(data.usage) };
}