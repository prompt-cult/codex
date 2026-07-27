import test from "node:test";
import assert from "node:assert/strict";
import { loadPolicy } from "./policy.mjs";
import { normalizeSpec, resolveSpec, specFields } from "./spec.mjs";

test("a tier-only spec defers entirely to the sysadmin policy table", async () => {
  const policy = await loadPolicy();
  const resolved = resolveSpec(policy, { role: "plan", tier: "eco" }, {});
  assert.equal(resolved.id, policy.tiers.eco.plan);
  assert.equal(resolved.tier, "eco");
  assert.equal(resolved.role, "plan");
});

test("a spec with no tier falls back to the policy defaultTier", async () => {
  const policy = await loadPolicy();
  const resolved = resolveSpec(policy, { role: "code" }, {});
  assert.equal(resolved.tier, policy.defaultTier);
  assert.equal(resolved.id, policy.tiers[policy.defaultTier].code);
});

test("build is accepted as an alias of the code role", async () => {
  const policy = await loadPolicy();
  const built = resolveSpec(policy, { role: "build", tier: "eco" }, {});
  assert.equal(built.role, "code");
  assert.equal(built.id, policy.tiers.eco.code);
});

test("an explicit spec resolves through the catalog, not the tier table", async () => {
  const policy = await loadPolicy();
  const resolved = resolveSpec(
    policy,
    {
      role: "plan",
      tier: "performance",
      vendor: "anthropic",
      model: "opus-performance",
      think: "high",
      contextWindow: 1000000,
    },
    {},
  );
  assert.equal(resolved.id, "opencode-zen:opus-performance");
  assert.equal(resolved.model, "claude-opus-5");
  assert.equal(resolved.tier, "performance");
  assert.equal(resolved.role, "plan");
});

test("model accepts the catalog reference, the catalog key, and the wire id", async () => {
  const policy = await loadPolicy();
  const spec = (model) => ({ role: "plan", tier: "performance", driver: "opencode-zen", model });
  for (const model of ["opencode-zen:opus-performance", "opus-performance", "claude-opus-5"]) {
    assert.equal(
      resolveSpec(policy, spec(model), {}).id,
      "opencode-zen:opus-performance",
      `alias ${model}`,
    );
  }
});

test("driver disambiguates a wire id that two drivers both carry", async () => {
  const policy = await loadPolicy();
  assert.throws(
    () => resolveSpec(policy, { role: "plan", tier: "balanced", model: "qwen3.6-plus" }, {}),
    /is ambiguous, it matched:.*opencode-go:qwen-code/s,
  );
  const zen = resolveSpec(
    policy,
    { role: "plan", tier: "balanced", driver: "opencode-zen", model: "qwen3.6-plus" },
    {},
  );
  assert.equal(zen.id, "opencode-zen:qwen-code");
  const go = resolveSpec(
    policy,
    { role: "plan", tier: "balanced", driver: "opencode-go", model: "qwen3.6-plus" },
    {},
  );
  assert.equal(go.id, "opencode-go:qwen-code");
});

test("a spec that matches nothing names the rejected candidates", async () => {
  const policy = await loadPolicy();
  assert.throws(
    () =>
      resolveSpec(
        policy,
        { role: "plan", tier: "performance", driver: "opencode-zen", vendor: "mistral" },
        {},
      ),
    /matched no catalog entry; role-capable candidates were:/,
  );
});

test("role capability is filtered before matching", async () => {
  const policy = await loadPolicy();
  // gpt-luna is plan/code/review only, so a summarize spec cannot reach it.
  assert.throws(
    () => resolveSpec(policy, { role: "summarize", tier: "performance", model: "gpt-luna" }, {}),
    /matched no catalog entry/,
  );
});

test("the cost gate applies to explicit specs exactly as to overrides", async () => {
  const policy = await loadPolicy();
  assert.throws(
    () => resolveSpec(policy, { role: "code", tier: "eco", model: "opus-performance" }, {}),
    /costClass 'premium' exceeds tier budget/,
  );
  const allowed = resolveSpec(
    policy,
    { role: "code", tier: "performance", model: "opus-performance" },
    {},
  );
  assert.equal(allowed.costClass, "premium");
});

test("the tier env override retargets the budget of an explicit spec", async () => {
  const policy = await loadPolicy();
  const resolved = resolveSpec(
    policy,
    { role: "code", tier: "eco", model: "opus-performance" },
    { THREADBOX_TIER: "performance" },
  );
  assert.equal(resolved.tier, "performance");
  assert.equal(resolved.id, "opencode-zen:opus-performance");

  const viaAlias = resolveSpec(
    policy,
    { role: "code", tier: "eco", model: "opus-performance" },
    { THREADBOX_PROFILE: "performance" },
  );
  assert.equal(viaAlias.tier, "performance");
});

test("an unknown tier is rejected on the explicit path too", async () => {
  const policy = await loadPolicy();
  assert.throws(
    () => resolveSpec(policy, { role: "code", tier: "luxury", model: "opus-performance" }, {}),
    /unknown ThreadBox tier 'luxury'/,
  );
});

test("normalizeSpec drops absent fields and lower-cases the declared ones", () => {
  assert.deepEqual(normalizeSpec({ role: "plan", tier: "eco" }), {});
  assert.deepEqual(
    normalizeSpec({
      role: "plan",
      tier: "performance",
      driver: "OpenCode-Zen",
      vendor: "Anthropic",
      model: "Claude-Opus-5",
      think: "High",
      contextWindow: 1000000,
    }),
    {
      driver: "opencode-zen",
      vendor: "anthropic",
      model: "claude-opus-5",
      think: "high",
      contextWindow: 1000000,
    },
  );
  assert.deepEqual(normalizeSpec({ role: "plan", vendor: "", model: null, think: undefined }), {});
});

test("normalizeSpec rejects malformed field values", () => {
  assert.throws(
    () => normalizeSpec({ role: "plan", contextWindow: 0 }),
    /invalid contextWindow '0', expected a positive integer/,
  );
  assert.throws(
    () => normalizeSpec({ role: "plan", contextWindow: 1.5 }),
    /invalid contextWindow '1.5'/,
  );
  assert.throws(
    () => normalizeSpec({ role: "plan", vendor: 7 }),
    /field 'vendor' must be a string, received 'number'/,
  );
});

test("a non-record spec and an unknown role are configuration errors", async () => {
  const policy = await loadPolicy();
  assert.throws(() => resolveSpec(policy, null, {}), /must be a record with at least a role/);
  assert.throws(() => resolveSpec(policy, "plan", {}), /must be a record with at least a role/);
  assert.throws(() => resolveSpec(policy, { role: "deploy" }, {}), /unknown ThreadBox role/);
});

test("specFields is the documented, defensively copied field order", () => {
  assert.deepEqual(specFields(), ["driver", "vendor", "model", "think", "contextWindow"]);
  const fields = specFields();
  fields.push("mutated");
  assert.deepEqual(specFields(), ["driver", "vendor", "model", "think", "contextWindow"]);
});
