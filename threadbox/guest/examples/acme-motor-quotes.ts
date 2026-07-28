/// The alpha graph: fill an Acme Motor Quotes application from a validated
/// submission, verify every write, and stop before the quote step.
///
/// The shape of this program is the argument. Capture, image transformation,
/// artifact selection, model invocation, the page write, and the verification
/// are separate nodes composed by the graph. No node knows what comes next,
/// and no model output reaches the page without passing a guard first.
///
/// The expensive path — capture, standardize, select, invoke a vision model —
/// runs only for the controls a deterministic lookup could not resolve. On
/// this form that is two of seventeen.

import {
  graph, input, tool, agent, transform, assertThat, validateWith,
  loop, branch, human, checkpoint, terminal, emitGraph,
  Node, p0, p1, p2,
} from "../assembly/dsl";

const SUBMISSION: string = "agent-dsl://schemas/submission/acme-motor-quotes/0.1.0";
const LOCALIZED: string = "agent-dsl://schemas/provider-output/localized-control/0.1.0";

/// The cheap arm: the control had an accessible name, so the deterministic
/// lookup already found it.
function useResolvedHandle(): Node {
  return transform("cheap-handle", ".memo[\"resolve\"].result.handle", p0());
}

/// The expensive arm: no accessible name, so look at the page.
///
/// Capture emits a raw artifact and stops. Standardize is a *separate* tool —
/// fusing them would bake one model's accepted dimensions into the capture
/// step. The guard then refuses to invoke a model unless a normalized image
/// artifact actually exists, which is what stops a raw capture reaching a
/// model-bound edge.
function localizeVisually(): Node {
  const shot = tool("capture", "capture.viewport",
    "{ navigationId: .memo[\"observe\"].result.navigationId }", p0());

  const normalized = tool("standardize", "image.standardize",
    "{ sourceArtifactId: .parents[0].files.items[0].artifactId, standardWidth: 1280, tileHeight: 900, overlap: 0 }",
    p1(shot));

  // Metadata-only selection: the graph picks images by semantic kind without
  // ever touching a byte.
  const images = transform("select-images",
    ".parents[0].files.items | map(select(.semanticKind == \"image\"))", p1(normalized));

  const usable = assertThat("images-present",
    "(.parents[0] | length) > 0", images);

  const located = agent("locate", "vision.primary",
    "agent-dsl-fs://prompts/locate-control@1",
    "{ target: .vars.field.description, files: .parents[0] }",
    LOCALIZED, p1(usable));

  const confident = assertThat("located-confidently",
    ".parents[0].result.confidence >= 0.7", located);

  // The model proposed a coordinate; the page still decides what is there.
  // A model verdict never overrides an observation.
  const atPoint = tool("locate-at-point", "dom.resolve_point",
    "{ navigationId: .memo[\"observe\"].result.navigationId, x: .parents[0].result.x, y: .parents[0].result.y }",
    p1(confident));

  const found = assertThat("point-hit-a-control",
    ".parents[0].result.resolved == true", atPoint);

  return transform("visual-handle", ".parents[0].result.handle", p1(found));
}

/// One field of the submission: resolve a target, propose a write, gate it,
/// apply it, then verify against a fresh read.
function enterField(): Node {
  const resolved = tool("resolve", "dom.resolve_accessible",
    "{ navigationId: .memo[\"observe\"].result.navigationId, accessibleName: .vars.field.accessibleName }",
    p0());

  const target = branch("pick-target", ".parents[0].result.resolved", resolved,
    useResolvedHandle, localizeVisually);

  const proposal = tool("propose", "form.propose_write",
    "{ navigationId: .memo[\"observe\"].result.navigationId, handle: .parents[0], value: .vars.field.value }",
    p1(target));

  const decision = human("gate", "form-write", ".parents[0].result", proposal);

  const approved = assertThat("approved",
    ".parents[0].result.decision == \"approve\"", decision);

  const applied = tool("apply", "form.apply",
    "{ proposalId: .memo[\"propose\"].result.proposalId, navigationId: .memo[\"observe\"].result.navigationId, handle: .memo[\"propose\"].result.handle, value: .vars.field.value, clearFirst: .memo[\"propose\"].result.clearFirst }",
    p1(approved));

  // Verification reads the page back. This is the postcondition that decides
  // whether the field was entered — not the tool's own claim to have applied.
  const proof = tool("verify", "form.verify",
    "{ navigationId: .memo[\"observe\"].result.navigationId, handle: .memo[\"propose\"].result.handle, expected: .vars.field.value }",
    p1(applied));

  const took = assertThat("write-took", ".parents[0].result.matches == true", proof);

  return checkpoint("field-done", "field-complete", ".vars.field.id", took);
}

/// One step of the form: enter every field mapped to it, then move on.
function enterStep(): Node {
  const observed = tool("observe", "page.observe", "{ freshness: \"new\" }", p0());

  const onExpectedStep = assertThat("step-matches",
    ".parents[0].result.step == .vars.step.index", observed);

  const fields = transform("fields-here",
    "$memo[\"load\"].document.answers | map(select(.step == $vars.step.index))",
    p1(onExpectedStep));

  const entered = loop("each-field", "field", ".parents[0]", 20, fields, enterField);

  const allEntered = assertThat("step-complete",
    "(.parents[0] | length) == $vars.step.count", entered);

  return branch("advance-if-more", ".vars.step.index < 3", allEntered,
    advancePage, stayPut);
}

function advancePage(): Node {
  return tool("advance", "page.advance", "{ freshness: \"new\" }", p0());
}

/// The final step deliberately does nothing. There is no submit tool to call,
/// and the run ends at the pre-submit boundary.
function stayPut(): Node {
  return transform("hold-at-submit", "\"awaiting_human_final_submit\"", p0());
}

export function main(): void {
  graph("acme-motor-quotes-alpha", "0.1.0");

  const submission = input("load", "submission", SUBMISSION);

  // Structural validation precedes any page interaction.
  const document = transform("document", ".parents[0].document", p1(submission));
  const valid = validateWith("submission-valid", SUBMISSION, document);

  // The step plan is derived from the submission, not hardcoded, so a
  // different mapping produces a different journey with no graph change.
  const steps = transform("plan-steps",
    "[ { index: 1, count: (.parents[0].answers | map(select(.step == 1)) | length) },"
    + "  { index: 2, count: (.parents[0].answers | map(select(.step == 2)) | length) },"
    + "  { index: 3, count: (.parents[0].answers | map(select(.step == 3)) | length) } ]",
    p1(valid));

  const done = loop("each-step", "step", ".parents[0]", 3, steps, enterStep);

  // Convergence is an independently checkable condition, not a claim.
  const everyStepDone = assertThat("all-steps-done", "(.parents[0] | length) == 3", done);

  terminal("stop", "success", "awaiting_human_final_submit", everyStepDone);

  emitGraph();
}
