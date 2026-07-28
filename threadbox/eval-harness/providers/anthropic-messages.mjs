import { normalizeUsage, postJson } from "./http.mjs";

export async function callAnthropicMessages(model, prompt, apiKey, fetchImpl) {
  const data = await postJson(
    `${model.baseUrl}/messages`,
    {
      model: model.model,
      messages: [{ role: "user", content: prompt }],
      max_tokens: model.maxOutputTokens,
      temperature: 0,
    },
    {
      "anthropic-version": "2023-06-01",
      "x-api-key": apiKey,
    },
    fetchImpl,
  );
  const output = data.content
    ?.filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("");
  if (!output) {
    throw new Error("Anthropic Messages response did not contain text output");
  }
  return { output, tokenUsage: normalizeUsage(data.usage) };
}