import { normalizeUsage, postJson } from "./http.mjs";

export async function callGemini(model, prompt, apiKey, fetchImpl) {
  const data = await postJson(
    `${model.baseUrl}/models/${model.model}:generateContent`,
    {
      contents: [{ role: "user", parts: [{ text: prompt }] }],
      generationConfig: {
        maxOutputTokens: model.maxOutputTokens,
        temperature: 0,
      },
    },
    { "x-goog-api-key": apiKey },
    fetchImpl,
  );
  const output = data.candidates?.[0]?.content?.parts
    ?.map((part) => part.text ?? "")
    .join("");
  if (!output) {
    throw new Error("Gemini response did not contain text output");
  }
  return {
    output,
    tokenUsage: normalizeUsage({
      inputTokens: data.usageMetadata?.promptTokenCount,
      outputTokens: data.usageMetadata?.candidatesTokenCount,
      totalTokens: data.usageMetadata?.totalTokenCount,
    }),
  };
}