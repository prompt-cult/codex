/// Resolves a builder specification record to one catalog entry.
///
/// The AssemblyScript builder (`sdk/assembly/models.d.ts`) never opens a
/// connection. It emits a flat record such as
/// `{ role: "plan", tier: "performance" }` or
/// `{ role: "plan", vendor: "anthropic", model: "opus-latest",
///    think: "medium", contextWindow: 1000000 }`.
///
/// This module is the only place that turns such a record into a
/// concrete driver + model. Two paths, chosen exhaustively:
///
///   - tier-only  -> delegate to the sysadmin policy table
///   - explicit   -> filter the merged catalog allowlist on the
///                   declared fields; zero matches and ambiguous
///                   matches are both hard configuration errors
import { normalizeRole, resolveModel, validateSelection } from "./policy.mjs";

/// Fields a spec may declare beyond `role`/`tier`, in the order they are
/// reported in error messages.
const SPEC_FIELDS = ["driver", "vendor", "model", "think", "contextWindow"];

/// Resolves `spec` against `policy`, honouring the same environment
/// overrides as `resolveModel`.
export function resolveSpec(policy, spec, environment = process.env) {
  if (!spec || typeof spec !== "object") {
    throw new Error("model spec must be a record with at least a role");
  }
  const role = normalizeRole(spec.role);
  const declared = normalizeSpec(spec);
  if (Object.keys(declared).length === 0) {
    return resolveModel(policy, { role, tier: spec.tier }, environment);
  }
  return resolveExplicit(policy, { ...spec, role }, declared, environment);
}

/// Lower-cases every declared string field and drops absent ones so the
/// filter only sees fields the caller actually asked for.
export function normalizeSpec(spec) {
  const declared = {};
  for (const field of SPEC_FIELDS) {
    const value = spec[field];
    if (value === undefined || value === null || value === "") {
      continue;
    }
    if (field === "contextWindow") {
      if (!Number.isInteger(value) || value <= 0) {
        throw new Error(
          `model spec has invalid contextWindow '${value}', expected a positive integer`,
        );
      }
      declared[field] = value;
      continue;
    }
    if (typeof value !== "string") {
      throw new Error(
        `model spec field '${field}' must be a string, received '${typeof value}'`,
      );
    }
    declared[field] = value.toLowerCase();
  }
  return declared;
}

function resolveExplicit(policy, spec, declared, environment) {
  const tierId =
    environment.THREADBOX_TIER ||
    environment.THREADBOX_PROFILE ||
    spec.tier ||
    policy.defaultTier;
  const tier = policy.tiers[tierId];
  if (!tier) {
    throw new Error(
      `unknown ThreadBox tier '${tierId}', expected one of: ${Object.keys(policy.tiers).join(", ")}`,
    );
  }
  const roleCapable = Object.values(policy.models).filter((entry) =>
    entry.roles.includes(spec.role),
  );
  const matches = roleCapable.filter((entry) => matchesSpec(entry, declared));
  if (matches.length === 0) {
    throw new Error(
      `model spec ${describeSpec(spec.role, declared)} matched no catalog entry; role-capable candidates were: ${describeCandidates(roleCapable)}`,
    );
  }
  if (matches.length > 1) {
    throw new Error(
      `model spec ${describeSpec(spec.role, declared)} is ambiguous, it matched: ${describeCandidates(matches)}`,
    );
  }
  const [match] = matches;
  // The cost gate is applied after resolution so an explicit spec cannot
  // spend premium budget under eco or balanced, exactly like an override.
  return {
    ...validateSelection(policy, match.id, spec.role, tier.allowedCostClasses),
    role: spec.role,
    tier: tierId,
  };
}

/// A catalog entry matches when every declared field matches. `model`
/// accepts the catalog reference, the catalog key, or the wire model id
/// so `Model.OpusLatest`, `opus-performance`, and `claude-opus-5` all
/// address the same entry.
function matchesSpec(entry, declared) {
  return Object.entries(declared).every(([field, value]) => {
    switch (field) {
      case "driver":
        return entry.driver.toLowerCase() === value;
      case "vendor":
        return entry.vendor.toLowerCase() === value;
      case "think":
        return entry.think.toLowerCase() === value;
      case "contextWindow":
        return entry.contextWindow === value;
      case "model":
        return modelAliases(entry).includes(value);
      default:
        throw new Error(`unhandled model spec field '${field}'`);
    }
  });
}

function modelAliases(entry) {
  const catalogKey = entry.id.slice(entry.driver.length + 1);
  return [entry.id, catalogKey, entry.model].map((alias) => alias.toLowerCase());
}

function describeSpec(role, declared) {
  const fields = Object.entries(declared).map(([key, value]) => `${key}=${value}`);
  return `{ role=${role}, ${fields.join(", ")} }`;
}

function describeCandidates(entries) {
  return entries
    .map((entry) => `${entry.id} (${entry.vendor}/${entry.model}, think=${entry.think}, context=${entry.contextWindow})`)
    .join("; ");
}

export function specFields() {
  return [...SPEC_FIELDS];
}
