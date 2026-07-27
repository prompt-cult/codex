import { callWire } from "./threadbox-provider.mjs";
import { loadPolicy, resolveModel } from "./policy.mjs";

process.loadEnvFile(new URL("../../.env", import.meta.url));

const policy = await loadPolicy();
const apiKey = process.env.OPENCODE_API_KEY;
if (!apiKey) {
  throw new Error("OPENCODE_API_KEY is required for Zen protocol smoke testing");
}

const cases = [
  ["zen-gpt-luna", "code", "performance"],
  ["zen-claude-haiku", "code", "eco"],
  ["zen-gemini-flash-lite", "code", "eco"],
  ["zen-free-open", "code", "eco"],
];

for (const [modelId, role, profile] of cases) {
  const model = resolveModel(policy, { role, modelId, profile });
  model.maxOutputTokens = 32;
  const result = await callWire(model, "Reply with exactly: THREADBOX_OK", apiKey);
  if (!result.output.includes("THREADBOX_OK")) {
    throw new Error(`${model.model} returned unexpected output`);
  }
  console.log(`zen/${model.model} (${model.wire}): OK`);
}
