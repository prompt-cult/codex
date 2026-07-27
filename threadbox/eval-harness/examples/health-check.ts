// Reference solution for kata-s6 (fetch and summarize service health).
//
// Referenced from prompts/system.md as a worked example. Graded the
// same way candidate solutions are graded: `asc --noEmit`, then
// structural fingerprint match against fingerprints/s6.json.
import { Flow, Uni, ThreadBox } from "../../sdk/assembly/threadbox.d";

// Named, top-level, non-closure callback -- required by the SDK's
// `Uni.map` contract (see threadbox.d.ts). Body is a placeholder;
// only the call shape is graded in Phase 1.
function summarizeHealth(responses: string[]): string {
  return "# Service health summary\n\n(operations agent summary placeholder)";
}

export function run(): void {
  // Three registered capabilities, called concurrently. There is no
  // way to construct an arbitrary URL here -- only names already
  // registered in the host's endpoint registry are resolvable, and an
  // unregistered name fails at graph-compile time on the host side.
  const health = Flow.endpoint("health");
  const metrics = Flow.endpoint("metrics-summary");
  const deployment = Flow.endpoint("deployment-status");

  // Join all three validated responses into a single array-valued node.
  const joined = Uni.joinAll<string>([health, metrics, deployment]);

  // Single operations agent call summarizes the joined, validated
  // responses into a current health report. This is the graph's only
  // agent call.
  const summary = joined.toAgent(
    "Given the joined health, metrics-summary, and deployment-status " +
      "responses, produce a current operations health summary."
  );

  ThreadBox.publish<string>(summary);
}
