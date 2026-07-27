/// Live smoke test: one cheap request per driver through the real wire
/// adapter, after the catalog + policy layers have validated the model.
///
/// Selection uses the current resolution contract:
///   resolveModel(policy, { role, tier, ref })
/// where `ref` is a `driverId:modelId` catalog reference.
///
/// Local drivers (`requiresApiKey: false`) report SKIP when the daemon
/// is not listening; a developer machine is not obliged to run one.
import { callWire, resolveApiKey } from "./threadbox-provider.mjs";
import { loadPolicy, resolveModel } from "./policy.mjs";

process.loadEnvFile(new URL("../../../.env", import.meta.url));

/// Driver alias -> [catalog ref, role]. The tier is always `eco` so the
/// smoke never spends premium budget.
const SMOKE_CASES = {
  zen: ["opencode-zen:free-open", "review"],
  go: ["opencode-go:qwen-code", "code"],
  mistral: ["mistral:small", "code"],
  groq: ["groq:fast", "summarize"],
  ollama: ["ollama:gemma4", "summarize"],
};
const DEFAULT_PROVIDERS = Object.keys(SMOKE_CASES);

/// Probe output budget per think level. A reasoning model spends the budget on
/// hidden reasoning before emitting any text, so `none` can afford 32 tokens
/// while `high` needs headroom to reach the visible answer.
const PROBE_TOKENS = { none: 32, low: 256, medium: 512, high: 1024 };

const selectedProviders = (process.env.THREADBOX_LIVE_PROVIDERS || DEFAULT_PROVIDERS.join(","))
  .split(",")
  .map((provider) => provider.trim())
  .filter(Boolean);
const policy = await loadPolicy();

for (const provider of selectedProviders) {
  const smokeCase = SMOKE_CASES[provider];
  if (!smokeCase) {
    throw new Error(
      `unsupported smoke provider '${provider}', expected one of: ${DEFAULT_PROVIDERS.join(", ")}`,
    );
  }
  const [ref, role] = smokeCase;
  const model = resolveModel(policy, { role, ref, tier: "eco" });
  const probeTokens = PROBE_TOKENS[model.think];
  if (probeTokens === undefined) {
    throw new Error(
      `no probe budget for think '${model.think}' on ${ref}, expected one of: ${Object.keys(PROBE_TOKENS).join(", ")}`,
    );
  }
  model.maxOutputTokens = Math.min(model.maxOutputTokens, probeTokens);
  const apiKey = resolveApiKey(model);
  try {
    await assertCatalogContains(model, apiKey);
    const result = await callWire(model, "Reply with exactly: THREADBOX_OK", apiKey);
    if (!result.output.includes("THREADBOX_OK")) {
      throw new Error(`${provider}/${model.model} returned unexpected output`);
    }
  } catch (error) {
    if (isUnreachable(error) && !model.requiresApiKey) {
      console.log(`${provider}/${model.model}: SKIP (not running)`);
      continue;
    }
    throw error;
  }
  console.log(`${provider}/${model.model}: OK`);
}

async function assertCatalogContains(model, apiKey) {
  const catalogUrl = `${model.baseUrl}/models`;
  const headers = apiKey ? { authorization: `Bearer ${apiKey}` } : {};
  const response = await fetch(catalogUrl, { headers });
  if (!response.ok) {
    throw new Error(`${model.driver} catalog returned HTTP ${response.status}`);
  }
  const catalog = await response.json();
  const modelIds = (catalog.data ?? []).map((entry) => entry.id);
  if (!modelIds.includes(model.model)) {
    throw new Error(
      `${model.driver} catalog does not include '${model.model}'; it serves: ${modelIds.join(", ")}`,
    );
  }
}

/// A local daemon that is not listening surfaces as a fetch TypeError
/// wrapping ECONNREFUSED / ENOTFOUND.
function isUnreachable(error) {
  const codes = [error?.cause?.code, error?.code].filter(Boolean);
  return codes.some((code) => code === "ECONNREFUSED" || code === "ENOTFOUND");
}
