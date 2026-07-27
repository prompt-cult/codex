import { readdir, readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { load as loadYaml } from "js-yaml";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_CATALOG_DIR = resolve(HERE, "../catalogs");
const DEFAULT_POLICY_PATH = resolve(HERE, "../policy.yaml");

const REQUIRED_ROLES = ["plan", "code", "review", "summarize"];
const ROLE_ALIASES = { build: "code" };
const WIRES = new Set(["responses", "messages", "chat-completions", "gemini"]);
/// Cost classes are ordered; `standard` is the band the balanced tier
/// needs, sitting below the premium price spike.
const COST_CLASSES = ["free", "low", "standard", "premium"];
const THINK_LEVELS = new Set(["none", "low", "medium", "high"]);

/// Loads every driver catalog in `catalogDir`, merges them into a single
/// allowlist keyed `driverId:modelId`, then validates the sysadmin
/// policy against that allowlist.
export async function loadPolicy(
  { catalogDir = DEFAULT_CATALOG_DIR, policyPath = DEFAULT_POLICY_PATH } = {},
) {
  const files = (await readdir(catalogDir))
    .filter((name) => name.endsWith(".yaml"))
    .sort();
  const catalogs = [];
  for (const name of files) {
    const contents = await readFile(join(catalogDir, name), "utf8");
    catalogs.push({ source: name, catalog: loadYaml(contents) });
  }
  const policy = loadYaml(await readFile(policyPath, "utf8"));
  return validatePolicy({ ...policy, models: mergeCatalogs(catalogs) });
}

/// Flattens driver catalogs into `driverId:modelId` entries. Duplicate
/// references across catalogs are a configuration error rather than a
/// silent last-one-wins.
export function mergeCatalogs(catalogs) {
  const models = {};
  for (const { source, catalog } of catalogs) {
    validateCatalog(source, catalog);
    for (const [modelId, entry] of Object.entries(catalog.models)) {
      const ref = `${catalog.driver}:${modelId}`;
      if (models[ref]) {
        throw new Error(
          `duplicate catalog reference '${ref}' declared in both '${models[ref].source}' and '${source}'`,
        );
      }
      models[ref] = validateCatalogEntry(ref, source, catalog, entry);
    }
  }
  return models;
}

function validateCatalog(source, catalog) {
  if (!catalog || typeof catalog !== "object") {
    throw new Error(`catalog '${source}' is empty or not a mapping`);
  }
  if (typeof catalog.driver !== "string" || catalog.driver.length === 0) {
    throw new Error(`catalog '${source}' must declare driver`);
  }
  if (!catalog.catalogVersion) {
    throw new Error(
      `catalog '${source}' must declare catalogVersion so stale catalogs are detectable`,
    );
  }
  if (typeof catalog.baseUrl !== "string" || catalog.baseUrl.length === 0) {
    throw new Error(`catalog '${source}' must declare baseUrl`);
  }
  const requiresApiKey = catalog.requiresApiKey !== false;
  if (requiresApiKey && !catalog.apiKeyEnv) {
    throw new Error(
      `catalog '${source}' must declare apiKeyEnv, or set requiresApiKey: false for a local driver`,
    );
  }
  if (!catalog.models || typeof catalog.models !== "object") {
    throw new Error(`catalog '${source}' must declare models`);
  }
}

function validateCatalogEntry(ref, source, catalog, entry) {
  if (!entry || typeof entry.model !== "string" || entry.model.length === 0) {
    throw new Error(`catalog entry '${ref}' must declare the wire model id`);
  }
  if (!WIRES.has(entry.wire)) {
    throw new Error(
      `catalog entry '${ref}' has unknown wire '${entry.wire}', expected one of: ${[...WIRES].join(", ")}`,
    );
  }
  if (!COST_CLASSES.includes(entry.costClass)) {
    throw new Error(
      `catalog entry '${ref}' has unknown costClass '${entry.costClass}', expected one of: ${COST_CLASSES.join(", ")}`,
    );
  }
  if (!THINK_LEVELS.has(entry.think)) {
    throw new Error(
      `catalog entry '${ref}' has unknown think '${entry.think}', expected one of: ${[...THINK_LEVELS].join(", ")}`,
    );
  }
  if (typeof entry.vendor !== "string" || entry.vendor.length === 0) {
    throw new Error(`catalog entry '${ref}' must declare vendor`);
  }
  if (!Number.isInteger(entry.contextWindow) || entry.contextWindow <= 0) {
    throw new Error(
      `catalog entry '${ref}' has invalid contextWindow '${entry.contextWindow}', expected a positive integer`,
    );
  }
  if (!Array.isArray(entry.roles) || entry.roles.length === 0) {
    throw new Error(`catalog entry '${ref}' must declare roles`);
  }
  for (const role of entry.roles) {
    if (!REQUIRED_ROLES.includes(role)) {
      throw new Error(
        `catalog entry '${ref}' declares unknown role '${role}', expected one of: ${REQUIRED_ROLES.join(", ")}`,
      );
    }
  }
  if (!Number.isInteger(entry.maxOutputTokens) || entry.maxOutputTokens <= 0) {
    throw new Error(
      `catalog entry '${ref}' has invalid maxOutputTokens '${entry.maxOutputTokens}'`,
    );
  }
  if (
    entry.supportsTemperature !== undefined &&
    typeof entry.supportsTemperature !== "boolean"
  ) {
    throw new Error(
      `catalog entry '${ref}' has non-boolean supportsTemperature '${entry.supportsTemperature}'`,
    );
  }
  return {
    ...entry,
    id: ref,
    source,
    driver: catalog.driver,
    catalogVersion: String(catalog.catalogVersion),
    baseUrl: catalog.baseUrl,
    apiKeyEnv: catalog.apiKeyEnv,
    requiresApiKey: catalog.requiresApiKey !== false,
  };
}

export function validatePolicy(policy) {
  if (!policy || policy.version !== 1) {
    throw new Error("policy.yaml must declare version: 1");
  }
  if (!policy.models || typeof policy.models !== "object") {
    throw new Error("policy must be merged with a non-empty catalog allowlist");
  }
  if (!policy.tiers || typeof policy.tiers !== "object") {
    throw new Error("policy.yaml must define tiers");
  }
  if (!policy.tiers[policy.defaultTier]) {
    throw new Error(
      `unknown defaultTier '${policy.defaultTier}', expected one of: ${Object.keys(policy.tiers).join(", ")}`,
    );
  }
  for (const [tierId, tier] of Object.entries(policy.tiers)) {
    const allowedCost = validateAllowedCostClasses(tierId, tier.allowedCostClasses);
    for (const role of REQUIRED_ROLES) {
      const ref = tier[role];
      if (!ref) {
        throw new Error(`tier '${tierId}' is missing role '${role}'`);
      }
      validateSelection(policy, ref, role, allowedCost);
    }
  }
  return policy;
}

function validateAllowedCostClasses(tierId, allowedCost) {
  if (!Array.isArray(allowedCost) || allowedCost.length === 0) {
    throw new Error(`tier '${tierId}' must declare allowedCostClasses`);
  }
  for (const costClass of allowedCost) {
    if (!COST_CLASSES.includes(costClass)) {
      throw new Error(
        `tier '${tierId}' has unknown allowedCostClass '${costClass}', expected one of: ${COST_CLASSES.join(", ")}`,
      );
    }
  }
  return allowedCost;
}

/// Normalizes a role, accepting `build` as an alias of `code`.
export function normalizeRole(role) {
  const normalized = ROLE_ALIASES[role] ?? role;
  if (!REQUIRED_ROLES.includes(normalized)) {
    throw new Error(
      `unknown ThreadBox role '${role}', expected one of: ${REQUIRED_ROLES.join(", ")}, build`,
    );
  }
  return normalized;
}

export function resolveModel(policy, options, environment = process.env) {
  const role = normalizeRole(options.role);
  const tierId =
    environment.THREADBOX_TIER ||
    environment.THREADBOX_PROFILE ||
    options.tier ||
    policy.defaultTier;
  const tier = policy.tiers[tierId];
  if (!tier) {
    throw new Error(
      `unknown ThreadBox tier '${tierId}', expected one of: ${Object.keys(policy.tiers).join(", ")}`,
    );
  }
  const allowedCost = validateAllowedCostClasses(tierId, tier.allowedCostClasses);
  const roleOverride = environment[`THREADBOX_ROLE_${role.toUpperCase()}`];
  const ref =
    environment.THREADBOX_FORCE_MODEL || roleOverride || options.ref || tier[role];
  return {
    ...validateSelection(policy, ref, role, allowedCost),
    role,
    tier: tierId,
  };
}

export function validateSelection(policy, ref, role, allowedCost) {
  const model = policy.models[ref];
  if (!model) {
    throw new Error(
      `catalog reference '${ref}' is not allowlisted; known references: ${Object.keys(policy.models).join(", ")}`,
    );
  }
  if (!model.roles.includes(role)) {
    throw new Error(
      `catalog reference '${ref}' does not support role '${role}', it declares: ${model.roles.join(", ")}`,
    );
  }
  if (!allowedCost.includes(model.costClass)) {
    throw new Error(
      `catalog reference '${ref}' costClass '${model.costClass}' exceeds tier budget [${allowedCost.join(", ")}]`,
    );
  }
  return model;
}

export function requiredRoles() {
  return [...REQUIRED_ROLES];
}

export function costClasses() {
  return [...COST_CLASSES];
}
