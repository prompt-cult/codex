# ThreadBox SDK -- kata-authoring reference

You are writing **AssemblyScript** source code against the ThreadBox
SDK below. Your only job is to express a static work graph using this
SDK's classes. You are not writing a runtime, a compiler, or host-side
code -- just the graph description.

## Hard rules

1. Import only from the SDK: your first line must be exactly
   `import { Flow, Uni, ThreadBox } from "../../sdk/assembly/threadbox.d";`
   No other imports are permitted.
2. Every callback passed to `.map(...)` or `Uni.dedupeArray(...)` must
   be a **named, top-level function declaration** -- never an arrow
   function or closure. This is a real constraint of the execution
   model (fresh-Wasmi-instance-per-callback dispatch by table index),
   not a style preference.
3. There is no way to branch, loop, or inspect a `Uni<T>`'s value from
   within your code. The graph is a fixed, static shape decided entirely
   by which SDK calls you make and how you chain them. Do not invent
   control-flow APIs that are not listed below.
4. Call exactly one `ThreadBox.publish(...)` per program -- this marks
   the graph's terminal output.
5. Do not invent SDK methods. If the task seems to need something not
   listed below, use the closest existing primitive (usually another
   `Flow.agent(...)` call, or a `.map(...)` with a named pure function).
6. Keep your solution under 200 lines.

## SDK reference

```typescript
export class Flow {
  /// Spawns a sandboxed agent with the given prompt. Counts toward any
  /// stated agent-call budget.
  static agent(prompt: string): Uni<string>;

  /// References a capability already registered in the host's endpoint
  /// registry by name (e.g. "health", "metrics-summary"). There is no
  /// way to construct an arbitrary URL -- an unregistered name fails
  /// at graph-compile time, not at runtime, and cannot be worked around.
  static endpoint(name: string): Uni<string>;
}

export class Uni<T> {
  /// Joins a fixed, statically-known list of nodes into one node that
  /// resolves once every input has resolved.
  static joinAll<T>(nodes: Uni<T>[]): Uni<T[]>;

  /// Pure, named-callback transform. `callback` must be a top-level
  /// named function, never a closure.
  map<U>(callback: (value: T) => U): Uni<U>;

  /// Removes duplicate elements from an array-valued node (e.g. the
  /// output of joinAll), keyed by a pure named per-element callback.
  static dedupeArray<T, K>(input: Uni<T[]>, keyOf: (item: T) => K): Uni<T[]>;

  /// Spawns a further agent whose context includes this node's
  /// resolved value plus the given prompt. This is the only way to
  /// hand upstream data into a further agent call. Counts toward the
  /// agent-call budget like Flow.agent.
  toAgent(prompt: string): Uni<string>;
}

export class ThreadBox {
  /// Marks the terminal output of the graph. Call exactly once.
  static publish<T>(result: Uni<T>): void;
}
```

## Worked example 1 -- parallel review (fork/join/rank)

Task: review the current branch without modifying it, run a
correctness reviewer and a security reviewer in parallel, join their
findings, remove duplicates, and ask a final reporting agent to rank
the remaining issues by severity. At most three agent calls.

```typescript
import { Flow, Uni, ThreadBox } from "../../sdk/assembly/threadbox.d";

function findingKey(finding: string): string {
  return finding;
}

function rankBySeverity(findings: string[]): string {
  return "# Review findings\n\n(ranked report placeholder)";
}

export function run(): void {
  const correctness = Flow.agent(
    "Review the current branch diff for correctness issues only. " +
      "List each finding with file and line reference. Do not modify anything."
  );
  const security = Flow.agent(
    "Review the current branch diff for security issues only. " +
      "List each finding with file and line reference. Do not modify anything."
  );

  const joined = Uni.joinAll<string>([correctness, security]);
  const deduped = Uni.dedupeArray<string, string>(joined, findingKey);
  const ranked = deduped.map<string>(rankBySeverity);

  ThreadBox.publish<string>(ranked);
}
```

## Worked example 2 -- service health fan-out

Task: call the registered health, metrics-summary, and
deployment-status endpoints in parallel. Join the validated responses
and ask an operations agent to produce a current health summary.

```typescript
import { Flow, Uni, ThreadBox } from "../../sdk/assembly/threadbox.d";

function summarizeHealth(responses: string[]): string {
  return "# Service health summary\n\n(operations agent summary placeholder)";
}

export function run(): void {
  const health = Flow.endpoint("health");
  const metrics = Flow.endpoint("metrics-summary");
  const deployment = Flow.endpoint("deployment-status");

  const joined = Uni.joinAll<string>([health, metrics, deployment]);
  const summary = joined.toAgent(
    "Given the joined health, metrics-summary, and deployment-status " +
      "responses, produce a current operations health summary."
  );

  ThreadBox.publish<string>(summary);
}
```

## Your task

{{ kataPrompt }}

## Output format

Reply with a single AssemblyScript code block (` ```typescript `
fenced) containing your full solution, starting with the required
import line and defining an exported `run(): void` function. Do not
include any explanation outside the code block.
