const ERROR_BODY_LIMIT = 1000;
const REQUEST_TIMEOUT_MS = 120000;

export async function postJson(url, body, headers = {}, fetchImpl = fetch) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS);
  try {
    const response = await fetchImpl(url, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...headers,
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    if (!response.ok) {
      const errorBody = (await response.text()).slice(0, ERROR_BODY_LIMIT);
      throw new Error(`provider returned HTTP ${response.status}: ${errorBody}`);
    }
    return await response.json();
  } finally {
    clearTimeout(timeout);
  }
}

export function normalizeUsage(usage = {}) {
  const prompt = usage.prompt_tokens ?? usage.input_tokens ?? usage.inputTokens ?? 0;
  const completion =
    usage.completion_tokens ?? usage.output_tokens ?? usage.outputTokens ?? 0;
  const total = usage.total_tokens ?? usage.totalTokens ?? prompt + completion;
  return { total, prompt, completion };
}