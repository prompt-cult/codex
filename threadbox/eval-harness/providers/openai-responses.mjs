import { normalizeUsage, postJson } from "./http.mjs";

export async function callOpenAiResponses(model, prompt, apiKey, fetchImpl) {
  const data = await postJson(
    `${model.baseUrl}/responses`,
    {
      model: model.model,
      input: prompt,
      max_output_tokens: model.maxOutputTokens,
      // No temperature: reasoning models (gpt-5.x) reject a temperature param.
    },
    { authorization: `Bearer ${apiKey}` },
    fetchImpl,
  );
  const output =
    data.output_text ??
    data.output
      ?.flatMap((item) => item.content ?? [])
      .filter((part) => part.type === "output_text")
      .map((part) => part.text)
      .join("");
  if (!output) {
    throw new Error("Responses API response did not contain text output");
  }
  return { output, tokenUsage: normalizeUsage(data.usage) };
}