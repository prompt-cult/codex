import test from "node:test";
import assert from "node:assert/strict";
import {
  costClasses,
  loadPolicy,
  mergeCatalogs,
  normalizeRole,
  requiredRoles,
  resolveModel,
  validatePolicy,
} from "./policy.mjs";

/// A minimal, self-contained catalog used by the negative tests so they
/// assert on rules rather than on whatever the shipped catalogs happen
/// to contain today.
function fixtureCatalog(overrides = {}) {
  return {
    source: "fixture.yaml",
    catalog: {
      driver: "fixture",
      catalogVersion: "2026-07-27",
      baseUrl: "https://example.invalid/v1",
      apiKeyEnv: "FIXTURE_API_KEY",
      models: {
        cheap: {
          model: "fixture-cheap",
          wire: "chat-completions",
          vendor: "fixture",
          contextWindow: 131072,
          think: "none",
          costClass: "low",
          roles: ["plan", "code", "review", "summarize"],
          maxOutputTokens: 2048,
        },
      },
      ...overrides,
    },
  };
}

test("configured policy resolves every role in every tier", async () => {
  const policy = await loadPolicy();
  for (const tier of Object.keys(policy.tiers)) {
    for (const role of requiredRoles()) {
      const resolved = resolveModel(policy, { tier, role }, {});
      assert.equal(resolved.tier, tier);
      assert.equal(resolved.role, role);
      assert.equal(resolved.id, policy.tiers[tier][role]);
      assert.ok(policy.tiers[tier].allowedCostClasses.includes(resolved.costClass));
    }
  }
});

test("every catalog entry carries its driver provenance", async () => {
  const policy = await loadPolicy();
  for (const [ref, entry] of Object.entries(policy.models)) {
    assert.equal(ref, `${entry.driver}:${ref.split(":")[1]}`);
    assert.ok(entry.catalogVersion, `${ref} must carry catalogVersion`);
    assert.ok(entry.baseUrl.startsWith("http"), `${ref} must carry baseUrl`);
  }
});

test("the local ollama driver needs no api key", async () => {
  const policy = await loadPolicy();
  const local = policy.models["ollama:gemma4"];
  assert.equal(local.requiresApiKey, false);
  assert.equal(local.baseUrl, "http://localhost:11434/v1");
});

test("build is accepted as an alias of the code role", async () => {
  const policy = await loadPolicy();
  assert.equal(normalizeRole("build"), "code");
  const built = resolveModel(policy, { tier: "eco", role: "build" }, {});
  const coded = resolveModel(policy, { tier: "eco", role: "code" }, {});
  assert.equal(built.id, coded.id);
  assert.equal(built.role, "code");
  assert.throws(() => normalizeRole("refactor"), /unknown ThreadBox role/);
});

test("tier env override wins over the requested tier", async () => {
  const policy = await loadPolicy();
  const viaTier = resolveModel(
    policy,
    { tier: "eco", role: "code" },
    { THREADBOX_TIER: "performance" },
  );
  assert.equal(viaTier.tier, "performance");
  assert.equal(viaTier.id, policy.tiers.performance.code);

  const viaProfileAlias = resolveModel(
    policy,
    { tier: "eco", role: "code" },
    { THREADBOX_PROFILE: "performance" },
  );
  assert.equal(viaProfileAlias.tier, "performance");
});

test("force override wins over role override and tier selection", async () => {
  const policy = await loadPolicy();
  const resolved = resolveModel(
    policy,
    { role: "code" },
    {
      THREADBOX_TIER: "performance",
      THREADBOX_FORCE_MODEL: "opencode-go:qwen-code",
      THREADBOX_ROLE_CODE: "mistral:large",
    },
  );
  assert.equal(resolved.id, "opencode-go:qwen-code");
  assert.equal(resolved.tier, "performance");
});

test("role override wins over the configured tier model", async () => {
  const policy = await loadPolicy();
  const resolved = resolveModel(
    policy,
    { tier: "eco", role: "review" },
    { THREADBOX_ROLE_REVIEW: "opencode-zen:free-open" },
  );
  assert.equal(resolved.id, "opencode-zen:free-open");
});

test("unknown and incompatible references are rejected", async () => {
  const policy = await loadPolicy();
  assert.throws(
    () => resolveModel(policy, { role: "code" }, { THREADBOX_FORCE_MODEL: "not-a-model" }),
    /not allowlisted/,
  );
  assert.throws(
    () =>
      resolveModel(
        policy,
        { tier: "performance", role: "summarize" },
        { THREADBOX_FORCE_MODEL: "opencode-zen:gpt-luna" },
      ),
    /does not support role 'summarize'/,
  );
  assert.throws(
    () => resolveModel(policy, { tier: "no-such-tier", role: "code" }, {}),
    /unknown ThreadBox tier/,
  );
});

test("premium references are rejected under eco even via override", async () => {
  const policy = await loadPolicy();
  assert.throws(
    () =>
      resolveModel(
        policy,
        { tier: "eco", role: "code" },
        { THREADBOX_FORCE_MODEL: "opencode-zen:opus-performance" },
      ),
    /costClass 'premium' exceeds tier budget/,
  );
  assert.throws(
    () =>
      resolveModel(
        policy,
        { tier: "balanced", role: "review" },
        { THREADBOX_ROLE_REVIEW: "opencode-go:kimi-performance" },
      ),
    /costClass 'premium' exceeds tier budget/,
  );
});

test("premium references resolve under the performance tier", async () => {
  const policy = await loadPolicy();
  const resolved = resolveModel(
    policy,
    { tier: "performance", role: "code" },
    { THREADBOX_FORCE_MODEL: "opencode-go:kimi-performance" },
  );
  assert.equal(resolved.id, "opencode-go:kimi-performance");
  assert.equal(resolved.costClass, "premium");
});

test("kimi-k3 is configured to omit temperature", async () => {
  const policy = await loadPolicy();
  const kimi = policy.models["opencode-go:kimi-performance"];
  assert.equal(kimi.model, "kimi-k3");
  assert.equal(kimi.supportsTemperature, false);
});

test("cost classes are ordered free to premium", () => {
  assert.deepEqual(costClasses(), ["free", "low", "standard", "premium"]);
});

test("malformed policies fail with descriptive errors", async () => {
  const policy = await loadPolicy();

  const missingRole = structuredClone(policy);
  delete missingRole.tiers.eco.plan;
  assert.throws(() => validatePolicy(missingRole), /missing role 'plan'/);

  const missingBudget = structuredClone(policy);
  delete missingBudget.tiers.eco.allowedCostClasses;
  assert.throws(() => validatePolicy(missingBudget), /must declare allowedCostClasses/);

  const unknownBudget = structuredClone(policy);
  unknownBudget.tiers.eco.allowedCostClasses = ["fre"];
  assert.throws(() => validatePolicy(unknownBudget), /unknown allowedCostClass 'fre'/);

  const overBudget = structuredClone(policy);
  overBudget.tiers.eco.code = "opencode-zen:opus-performance";
  assert.throws(() => validatePolicy(overBudget), /exceeds tier budget/);

  const badDefault = structuredClone(policy);
  badDefault.defaultTier = "luxury";
  assert.throws(() => validatePolicy(badDefault), /unknown defaultTier 'luxury'/);

  const badVersion = structuredClone(policy);
  badVersion.version = 2;
  assert.throws(() => validatePolicy(badVersion), /must declare version: 1/);
});

test("catalogs must declare a version so staleness is detectable", () => {
  const withoutVersion = fixtureCatalog();
  delete withoutVersion.catalog.catalogVersion;
  assert.throws(() => mergeCatalogs([withoutVersion]), /must declare catalogVersion/);
});

test("catalogs must declare apiKeyEnv unless they are local", () => {
  const remote = fixtureCatalog();
  delete remote.catalog.apiKeyEnv;
  assert.throws(() => mergeCatalogs([remote]), /must declare apiKeyEnv/);

  const local = fixtureCatalog({ requiresApiKey: false });
  delete local.catalog.apiKeyEnv;
  const models = mergeCatalogs([local]);
  assert.equal(models["fixture:cheap"].requiresApiKey, false);
});

test("duplicate catalog references are a configuration error", () => {
  assert.throws(
    () => mergeCatalogs([fixtureCatalog(), fixtureCatalog()]),
    /duplicate catalog reference 'fixture:cheap'/,
  );
});

test("catalog entries are validated field by field", () => {
  const cases = [
    ["model", undefined, /must declare the wire model id/],
    ["wire", "grpc", /unknown wire 'grpc'/],
    ["costClass", "cheap", /unknown costClass 'cheap'/],
    ["think", "hard", /unknown think 'hard'/],
    ["vendor", undefined, /must declare vendor/],
    ["contextWindow", 0, /invalid contextWindow/],
    ["roles", [], /must declare roles/],
    ["roles", ["deploy"], /unknown role 'deploy'/],
    ["maxOutputTokens", -1, /invalid maxOutputTokens/],
    ["supportsTemperature", "false", /non-boolean supportsTemperature/],
  ];
  for (const [field, value, expected] of cases) {
    const broken = fixtureCatalog();
    if (value === undefined) {
      delete broken.catalog.models.cheap[field];
    } else {
      broken.catalog.models.cheap[field] = value;
    }
    assert.throws(() => mergeCatalogs([broken]), expected, `field ${field}`);
  }
});
