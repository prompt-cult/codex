import test from "node:test";
import assert from "node:assert/strict";
import { callWire } from "./threadbox-provider.mjs";

function mockedFetch(responseBody, status = 200) {
  const calls = [];
  const fetchImpl = async (url, options) => {
    calls.push({ url, options });
    return new Response(JSON.stringify(responseBody), {
      status,
      headers: { "content-type": "application/json" },
    });
  };
  return { calls, fetchImpl };
}

function model(wire) {
  return {
    baseUrl: "https://provider.example/v1",
    model: "example-model",
    maxOutputTokens: 123,
    wire,
  };
}

test("OpenAI Chat adapter sends capped deterministic request", async () => {
  const mock = mockedFetch({
    choices: [{ message: { content: "chat output" } }],
    usage: { prompt_tokens: 2, completion_tokens: 3, total_tokens: 5 },
  });
  const result = await callWire(model("chat-completions"), "hello", "secret", mock.fetchImpl);
  assert.deepEqual(result, {
    output: "chat output",
    tokenUsage: { total: 5, prompt: 2, completion: 3 },
  });
  assert.equal(mock.calls[0].url, "https://provider.example/v1/chat/completions");
  const body = JSON.parse(mock.calls[0].options.body);
  assert.deepEqual(body, {
    model: "example-model",
    messages: [{ role: "user", content: "hello" }],
    max_tokens: 123,
    temperature: 0,
  });
});

test("Anthropic Messages adapter normalizes text and usage", async () => {
  const mock = mockedFetch({
    content: [{ type: "text", text: "message output" }],
    usage: { input_tokens: 4, output_tokens: 6 },
  });
  const result = await callWire(model("messages"), "hello", "secret", mock.fetchImpl);
  assert.deepEqual(result, {
    output: "message output",
    tokenUsage: { total: 10, prompt: 4, completion: 6 },
  });
  assert.equal(mock.calls[0].url, "https://provider.example/v1/messages");
  assert.equal(mock.calls[0].options.headers["x-api-key"], "secret");
});

test("Responses adapter extracts output array text", async () => {
  const mock = mockedFetch({
    output: [{ content: [{ type: "output_text", text: "responses output" }] }],
    usage: { input_tokens: 7, output_tokens: 8, total_tokens: 15 },
  });
  const result = await callWire(model("responses"), "hello", "secret", mock.fetchImpl);
  assert.deepEqual(result, {
    output: "responses output",
    tokenUsage: { total: 15, prompt: 7, completion: 8 },
  });
  assert.equal(mock.calls[0].url, "https://provider.example/v1/responses");
});

test("Gemini adapter uses generateContent and normalizes usage", async () => {
  const mock = mockedFetch({
    candidates: [{ content: { parts: [{ text: "gemini output" }] } }],
    usageMetadata: {
      promptTokenCount: 3,
      candidatesTokenCount: 5,
      totalTokenCount: 8,
    },
  });
  const result = await callWire(model("gemini"), "hello", "secret", mock.fetchImpl);
  assert.deepEqual(result, {
    output: "gemini output",
    tokenUsage: { total: 8, prompt: 3, completion: 5 },
  });
  assert.equal(
    mock.calls[0].url,
    "https://provider.example/v1/models/example-model:generateContent",
  );
  assert.equal(mock.calls[0].options.headers["x-goog-api-key"], "secret");
});

test("provider errors are bounded", async () => {
  const secret = "super-secret-key";
  const mock = mockedFetch({ detail: "x".repeat(2000) }, 400);
  await assert.rejects(
    () => callWire(model("chat-completions"), "hello", secret, mock.fetchImpl),
    (error) => {
      assert.match(error.message, /^provider returned HTTP 400:/);
      assert.equal(error.message.includes(secret), false);
      assert.ok(error.message.length < 1100);
      return true;
    },
  );
});
test("Chat adapter omits temperature when supportsTemperature is false", async () => {
  const mock = mockedFetch({
    choices: [{ message: { content: "reasoning output" } }],
    usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
  });
  const reasoningModel = { ...model("chat-completions"), supportsTemperature: false };
  await callWire(reasoningModel, "hello", "secret", mock.fetchImpl);
  const body = JSON.parse(mock.calls[0].options.body);
  assert.equal("temperature" in body, false);
  assert.deepEqual(body, {
    model: "example-model",
    messages: [{ role: "user", content: "hello" }],
    max_tokens: 123,
  });
});

test("Chat adapter still sends temperature when the flag is absent or true", async () => {
  for (const supportsTemperature of [undefined, true]) {
    const mock = mockedFetch({
      choices: [{ message: { content: "out" } }],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    });
    const m = { ...model("chat-completions") };
    if (supportsTemperature !== undefined) m.supportsTemperature = supportsTemperature;
    await callWire(m, "hello", "secret", mock.fetchImpl);
    assert.equal(JSON.parse(mock.calls[0].options.body).temperature, 0);
  }
});
