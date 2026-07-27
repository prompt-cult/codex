import { callWire } from "./threadbox-provider.mjs";
import { loadPolicy, resolveModel } from "./policy.mjs";

process.loadEnvFile(new URL("../../../.env", import.meta.url));

const DEFAULT_PROVIDERS = ["zen", "go", "mistral", "groq"];
const SMOKE_MODELS = {
  zen: "zen-free-open",
  go: "go-qwen-code",
  mistral: "mistral-small",
  groq: "groq-fast",
};
const SMOKE_ROLES = {
  zen: "review",
  go: "code",
  mistral: "code",
  groq: "summarize",
};
const selectedProviders = (process.env.THREADBOX_LIVE_PROVIDERS || DEFAULT_PROVIDERS.join(","))
  .split(",")
  .map((provider) => provider.trim())
  .filter(Boolean);
const policy = await loadPolicy();

for (const provider of selectedProviders) {
  const modelId = SMOKE_MODELS[provider];
  const role = SMOKE_ROLES[provider];
  if (!modelId) {
    throw new Error(`unsupported smoke provider '${provider}'`);
  }
  const model = resolveModel(policy, { role, modelId, profile: "eco" });
  model.maxOutputTokens = 32;
  const apiKey = process.env[model.apiKeyEnv];
  if (!apiKey) {
    throw new Error(`${model.apiKeyEnv} is required for ${provider} smoke testing`);
  }
  await assertCatalogContains(model, apiKey);
  const result = await callWire(model, "Reply with exactly: THREADBOX_OK", apiKey);
  if (!result.output.includes("THREADBOX_OK")) {
    throw new Error(`${provider}/${model.model} returned unexpected output`);
  }
  console.log(`${provider}/${model.model}: OK`);
}

async function assertCatalogContains(model, apiKey) {
  const catalogUrl = `${model.baseUrl}/models`;
  const response = await fetch(catalogUrl, {
    headers: { authorization: `Bearer ${apiKey}` },
  });
  if (!response.ok) {
    throw new Error(`${model.provider} catalog returned HTTP ${response.status}`);
  }
  const catalog = await response.json();
  const modelIds = (catalog.data ?? []).map((entry) => entry.id);
  if (!modelIds.includes(model.model)) {
    throw new Error(`${model.provider} catalog does not include '${model.model}'`);
  }
}
