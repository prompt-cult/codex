// Structural fingerprint matcher for ThreadBox Phase 1 katas.
//
// Deliberately loose: this checks that a required *set* of SDK calls
// appears with a plausible count, not an exact AST/token match --
// stylistically different but equally valid solutions must not be
// penalized. See grade.mjs for how this fits into the 3-stage gate
// (asc --noEmit -> structure -> lint).

const CALL_PATTERNS = {
  "Flow.agent": /Flow\.agent\s*\(/g,
  "Flow.endpoint": /Flow\.endpoint\s*\(/g,
  "Uni.joinAll": /Uni\.joinAll\s*[<(]/g,
  "Uni.dedupeArray": /Uni\.dedupeArray\s*[<(]/g,
  "Uni.map": /\.map\s*[<(]/g,
  "toAgent": /\.toAgent\s*\(/g,
  "ThreadBox.publish": /ThreadBox\.publish\s*[<(]/g,
};

function countMatches(source, name) {
  const pattern = CALL_PATTERNS[name];
  if (!pattern) {
    throw new Error(
      `check-structure.mjs: unknown call pattern "${name}" -- add it to CALL_PATTERNS`
    );
  }
  // Patterns are global (`g`); re-match from scratch each call so
  // callers can invoke countMatches repeatedly without lastIndex bugs.
  const matches = source.match(pattern);
  return matches ? matches.length : 0;
}

/// Checks `source` against a fingerprint spec (see fingerprints/*.json
/// for the schema in use: requiredCalls, requiredAnyOf, forbiddenCalls,
/// maxAgentCalls, publishCount). Returns `{ pass, details }` where
/// `details` is a list of human-readable mismatch explanations.
export function checkStructure(source, fingerprint) {
  const details = [];

  for (const [name, range] of Object.entries(fingerprint.requiredCalls || {})) {
    const count = countMatches(source, name);
    if (range.min !== undefined && count < range.min) {
      details.push(`${name}: found ${count}, need at least ${range.min}`);
    }
    if (range.max !== undefined && count > range.max) {
      details.push(`${name}: found ${count}, allowed at most ${range.max}`);
    }
  }

  for (const group of fingerprint.requiredAnyOf || []) {
    const total = group.reduce((sum, name) => sum + countMatches(source, name), 0);
    if (total < 1) {
      details.push(`none of [${group.join(", ")}] found, at least one required`);
    }
  }

  for (const name of fingerprint.forbiddenCalls || []) {
    const count = countMatches(source, name);
    if (count > 0) {
      details.push(`${name}: found ${count}, this kata forbids it`);
    }
  }

  if (fingerprint.publishCount) {
    const count = countMatches(source, "ThreadBox.publish");
    const { min = 1, max = 1 } = fingerprint.publishCount;
    if (count < min || count > max) {
      details.push(`ThreadBox.publish: found ${count}, expected between ${min} and ${max}`);
    }
  }

  if (fingerprint.maxAgentCalls !== undefined) {
    const agentCalls = countMatches(source, "Flow.agent") + countMatches(source, "toAgent");
    if (agentCalls > fingerprint.maxAgentCalls) {
      details.push(
        `total agent calls (Flow.agent + toAgent): found ${agentCalls}, allowed at most ${fingerprint.maxAgentCalls}`
      );
    }
  }

  return { pass: details.length === 0, details };
}
