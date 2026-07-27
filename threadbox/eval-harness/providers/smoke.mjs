/// Live smoke test: one cheap request per driver through the real wire
/// adapter, after the catalog + policy layers have validated the model.
///
/// Selection uses the current resolution contract:
///   resolveModel(policy, { role, tier, ref })
/// where `ref` is a `driverId:modelId` catalog reference.
///
/// Local drivers (`requiresApiKey: false`) report SKIP when the daemon
/// is not listening; a developer machine is not obliged to run one.
///
/// Every selected driver reports exactly one line: OK, SKIP (not running /
/// rate limited), or FAIL with the error. A 429 is retried with backoff
/// before being treated as a SKIP, and any FAIL sets a non-zero exit code.
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

/// Rate limits are transient: retry the probe with backoff before treating
/// the driver as rate limited. Cheap probes keep this inexpensive.
const RATE_LIMIT_ATTEMPTS = 3;
const RATE_LIMIT_BACKOFF_MS = 15_000;

/// Every driver reports a result so one failure never hides the rest.
const results = [];
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
  const label = `${provider}/${model.model}`;
  try {
    await assertCatalogContains(model, apiKey);
    await probeWithRetry(model, apiKey, label);
  } catch (error) {
    if (isUnreachable(error) && !model.requiresApiKey) {
      results.push({ label, status: "SKIP", detail: "not running" });
    } else if (isRateLimited(error)) {
      results.push({ label, status: "SKIP", detail: "rate limited" });
    } else {
      results.push({ label, status: "FAIL", detail: errorMessage(error) });
    }
    continue;
  }
  results.push({ label, status: "OK", detail: "" });
}

let failures = 0;
for (const { label, status, detail } of results) {
  const suffix = detail ? ` (${detail})` : "";
  console.log(`${label}: ${status}${suffix}`);
  if (status === "FAIL") failures += 1;
}
if (failures > 0) {
  process.exitCode = 1;
}

/// One probe attempt, retrying only on HTTP 429. Any other error propagates
/// immediately so a real regression is not masked by retries.
async function probeWithRetry(model, apiKey, label) {
  for (let attempt = 1; ; attempt += 1) {
    try {
      const result = await callWire(model, "Reply with exactly: THREADBOX_OK", apiKey);
      if (!result.output.includes("THREADBOX_OK")) {
        throw new Error(`${label} returned unexpected output`);
      }
      return;
    } catch (error) {
      if (isRateLimited(error) && attempt < RATE_LIMIT_ATTEMPTS) {
        await sleep(RATE_LIMIT_BACKOFF_MS * attempt);
        continue;
      }
      throw error;
    }
  }
}

/// The adapters surface rate limiting as `provider returned HTTP 429` from
/// postJson; match the status code rather than a vendor-specific message.
function isRateLimited(error) {
  return errorMessage(error).includes("HTTP 429");
}

function errorMessage(error) {
  return error instanceof Error ? error.message : String(error);
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
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
