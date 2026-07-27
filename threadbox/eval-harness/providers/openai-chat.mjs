import { normalizeUsage, postJson } from "./http.mjs";

export async function callOpenAiChat(model, prompt, apiKey, fetchImpl) {
  const data = await postJson(
    `${model.baseUrl}/chat/completions`,
    {
      model: model.model,
      messages: [{ role: "user", content: prompt }],
      max_tokens: model.maxOutputTokens,
      temperature: 0,
    },
    { authorization: `Bearer ${apiKey}` },
    fetchImpl,
  );
  const output = data.choices?.[0]?.message?.content;
  if (typeof output !== "string") {
    throw new Error("Chat Completions response did not contain text output");
  }
  return { output, tokenUsage: normalizeUsage(data.usage) };
}