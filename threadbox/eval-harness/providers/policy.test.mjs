import test from "node:test";
import assert from "node:assert/strict";
import { loadPolicy, requiredRoles, resolveModel, validatePolicy } from "./policy.mjs";

test("configured policy resolves every role in every profile", async () => {
  const policy = await loadPolicy();
  for (const profile of Object.keys(policy.profiles)) {
    for (const role of requiredRoles()) {
      const resolved = resolveModel(policy, { profile, role }, {});
      assert.equal(resolved.profile, profile);
      assert.equal(resolved.id, policy.profiles[profile][role]);
      assert.equal(resolved.enabled, true);
    }
  }
});

test("global override wins over role and profile selections", async () => {
  const policy = await loadPolicy();
  const environment = {
    THREADBOX_PROFILE: "performance",
    THREADBOX_FORCE_MODEL: "go-qwen-code",
    THREADBOX_ROLE_CODE: "mistral-large",
  };
  const resolved = resolveModel(policy, { role: "code" }, environment);
  assert.equal(resolved.id, "go-qwen-code");
  assert.equal(resolved.profile, "performance");
});

test("role override wins over configured profile model", async () => {
  const policy = await loadPolicy();
  const resolved = resolveModel(
    policy,
    { profile: "eco", role: "review" },
    { THREADBOX_ROLE_REVIEW: "mistral-small" },
  );
  assert.equal(resolved.id, "mistral-small");
});

test("unknown and incompatible overrides are rejected", async () => {
  const policy = await loadPolicy();
  assert.throws(
    () =>
      resolveModel(policy, { role: "code" }, { THREADBOX_FORCE_MODEL: "not-a-model" }),
    /not allowlisted/,
  );
  assert.throws(
    () =>
      resolveModel(policy, { role: "summarize" }, { THREADBOX_FORCE_MODEL: "zen-gpt-luna" }),
    /does not support role/,
  );
});

test("disabled models are rejected before use", async () => {
  const policy = await loadPolicy();
  const disabled = structuredClone(policy);
  disabled.models["go-qwen-code"].enabled = false;
  assert.throws(
    () => resolveModel(disabled, { role: "code" }, { THREADBOX_FORCE_MODEL: "go-qwen-code" }),
    /disabled/,
  );
});

test("malformed policies fail with descriptive errors", async () => {
  const policy = await loadPolicy();
  const malformed = structuredClone(policy);
  delete malformed.profiles.eco.plan;
  assert.throws(() => validatePolicy(malformed), /missing role 'plan'/);
});

test("premium models are rejected under the eco profile even via override", async () => {
  const policy = await loadPolicy();
  // go-kimi-performance supports 'code' (roles: [plan, code, review]) so this
  // reaches the cost-gate rather than failing on a role mismatch.
  assert.throws(
    () =>
      resolveModel(
        policy,
        { profile: "eco", role: "code" },
        { THREADBOX_FORCE_MODEL: "go-kimi-performance" },
      ),
    /costClass 'premium' exceeds profile budget/,
  );
  assert.throws(
    () =>
      resolveModel(
        policy,
        { profile: "eco", role: "review" },
        { THREADBOX_ROLE_REVIEW: "zen-gpt-luna" },
      ),
    /costClass 'premium' exceeds profile budget/,
  );
});

test("premium models resolve under the performance profile", async () => {
  const policy = await loadPolicy();
  const resolved = resolveModel(
    policy,
    { profile: "performance", role: "code" },
    { THREADBOX_FORCE_MODEL: "go-kimi-performance" },
  );
  assert.equal(resolved.id, "go-kimi-performance");
  assert.equal(resolved.costClass, "premium");
});

test("profiles must declare allowedCostClasses", async () => {
  const policy = await loadPolicy();
  const malformed = structuredClone(policy);
  delete malformed.profiles.eco.allowedCostClasses;
  assert.throws(() => validatePolicy(malformed), /must declare allowedCostClasses/);
});

test("validatePolicy rejects an over-budget configured model", async () => {
  const policy = await loadPolicy();
  const malformed = structuredClone(policy);
  // eco allows only [free, low]; mistral-large is premium.
  malformed.profiles.eco.code = "mistral-large";
  assert.throws(() => validatePolicy(malformed), /exceeds profile budget/);
});

test("resolveModel rejects unknown allowedCostClasses values", async () => {
  const policy = await loadPolicy();
  const malformed = structuredClone(policy);
  malformed.profiles.eco.allowedCostClasses = ["fre"];
  assert.throws(
    () => resolveModel(malformed, { profile: "eco", role: "code" }, {}),
    /unknown allowedCostClass/,
  );
});
