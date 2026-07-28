import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { load as loadYaml } from "js-yaml";

const REQUIRED_ROLES = ["plan", "code", "review", "summarize"];
const PROVIDERS = new Set(["zen", "go", "mistral", "groq"]);
const WIRES = new Set(["responses", "messages", "chat-completions", "gemini"]);
const COST_CLASSES = new Set(["free", "low", "premium"]);
const DEFAULT_POLICY_PATH = resolve(
  dirname(fileURLToPath(import.meta.url)),
  "../models.yaml",
);

export async function loadPolicy(policyPath = DEFAULT_POLICY_PATH) {
  const contents = await readFile(policyPath, "utf8");
  return validatePolicy(loadYaml(contents));
}

export function validatePolicy(policy) {
  if (!policy || policy.version !== 1) {
    throw new Error("models.yaml must declare version: 1");
  }
  if (!policy.models || typeof policy.models !== "object") {
    throw new Error("models.yaml must define models");
  }
  if (!policy.profiles || typeof policy.profiles !== "object") {
    throw new Error("models.yaml must define profiles");
  }
  if (!policy.profiles[policy.defaultProfile]) {
    throw new Error(`unknown defaultProfile '${policy.defaultProfile}'`);
  }

  for (const [modelId, model] of Object.entries(policy.models)) {
    validateModel(modelId, model);
  }
  for (const [profileId, profile] of Object.entries(policy.profiles)) {
    if (!Array.isArray(profile.allowedCostClasses) || profile.allowedCostClasses.length === 0) {
      throw new Error(`profile '${profileId}' must declare allowedCostClasses`);
    }
    for (const costClass of profile.allowedCostClasses) {
      if (!COST_CLASSES.has(costClass)) {
        throw new Error(`profile '${profileId}' has unknown allowedCostClass '${costClass}'`);
      }
    }
    for (const role of REQUIRED_ROLES) {
      const modelId = profile[role];
      if (!modelId) {
        throw new Error(`profile '${profileId}' is missing role '${role}'`);
      }
      validateSelection(policy, modelId, role, profile.allowedCostClasses);
    }
  }
  return policy;
}

function validateModel(modelId, model) {
  if (!PROVIDERS.has(model.provider)) {
    throw new Error(`model '${modelId}' has unknown provider '${model.provider}'`);
  }
  if (!WIRES.has(model.wire)) {
    throw new Error(`model '${modelId}' has unknown wire '${model.wire}'`);
  }
  if (!COST_CLASSES.has(model.costClass)) {
    throw new Error(`model '${modelId}' has unknown costClass '${model.costClass}'`);
  }
  if (!model.model || !model.baseUrl || !model.apiKeyEnv) {
    throw new Error(`model '${modelId}' is missing connection metadata`);
  }
  if (!Array.isArray(model.roles) || model.roles.length === 0) {
    throw new Error(`model '${modelId}' must declare roles`);
  }
  if (!Number.isInteger(model.maxOutputTokens) || model.maxOutputTokens <= 0) {
    throw new Error(`model '${modelId}' has invalid maxOutputTokens`);
  }
}

export function resolveModel(policy, options, environment = process.env) {
  const role = options.role;
  if (!REQUIRED_ROLES.includes(role)) {
    throw new Error(`unknown ThreadBox role '${role}'`);
  }
  const profileId = environment.THREADBOX_PROFILE || options.profile || policy.defaultProfile;
  const profile = policy.profiles[profileId];
  if (!profile) {
    throw new Error(`unknown ThreadBox profile '${profileId}'`);
  }
  const allowedCost = profile.allowedCostClasses;
  if (!Array.isArray(allowedCost) || allowedCost.length === 0) {
    throw new Error(`profile '${profileId}' must declare allowedCostClasses`);
  }
  for (const costClass of allowedCost) {
    if (!COST_CLASSES.has(costClass)) {
      throw new Error(`profile '${profileId}' has unknown allowedCostClass '${costClass}'`);
    }
  }
  const roleOverride = environment[`THREADBOX_ROLE_${role.toUpperCase()}`];
  const modelId =
    environment.THREADBOX_FORCE_MODEL ||
    roleOverride ||
    options.modelId ||
    profile[role];
  return {
    id: modelId,
    profile: profileId,
    ...validateSelection(policy, modelId, role, allowedCost),
  };
}

function validateSelection(policy, modelId, role, allowedCost) {
  const model = policy.models[modelId];
  if (!model) {
    throw new Error(`model '${modelId}' is not allowlisted`);
  }
  if (model.enabled !== true) {
    throw new Error(`model '${modelId}' is disabled`);
  }
  if (!model.roles.includes(role)) {
    throw new Error(`model '${modelId}' does not support role '${role}'`);
  }
  if (!allowedCost.includes(model.costClass)) {
    throw new Error(
      `model '${modelId}' costClass '${model.costClass}' exceeds profile budget [${allowedCost.join(", ")}]`,
    );
  }
  return model;
}

export function requiredRoles() {
  return [...REQUIRED_ROLES];
}
