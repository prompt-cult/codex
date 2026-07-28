// Reference solution for kata-s1 (parallel review of the current change).
//
// Referenced from prompts/system.md as a worked example. Graded the
// same way candidate solutions are graded: `asc --noEmit`, then
// structural fingerprint match against fingerprints/s1.json.
import { Flow, Uni, ThreadBox } from "../../sdk/assembly/threadbox.d";

// Named, top-level, non-closure callbacks -- required by the SDK's
// `Uni.map` / `Uni.dedupeArray` contract (see threadbox.d.ts). Bodies
// are placeholders; only the call shape is graded in Phase 1.
function findingKey(finding: string): string {
  return finding;
}

function rankBySeverity(findings: string[]): string {
  return "# Review findings\n\n(ranked report placeholder)";
}

export function run(): void {
  // Two independent reviewers in parallel -- 2 of the 3 allowed agent calls.
  const correctness = Flow.agent(
    "Review the current branch diff for correctness issues only. " +
      "List each finding with file and line reference. Do not modify anything."
  );
  const security = Flow.agent(
    "Review the current branch diff for security issues only. " +
      "List each finding with file and line reference. Do not modify anything."
  );

  // Join both reviewers' findings into a single array-valued node.
  const joined = Uni.joinAll<string>([correctness, security]);

  // Remove duplicate findings reported by both reviewers.
  const deduped = Uni.dedupeArray<string, string>(joined, findingKey);

  // Final reporting agent -- 3rd and last allowed agent call -- ranks
  // the remaining issues by severity and formats the Markdown report.
  const ranked = deduped.map<string>(rankBySeverity);

  ThreadBox.publish<string>(ranked);
}
