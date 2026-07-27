# ThreadBox Agent Guidance

## Scope

This subtree owns the ThreadBox DSL, evaluation harness, provider
policy, and future runtime. It does not extend Codex provider routing.

## Provider policy

- Keep provider URLs, credentials, and concrete model selection out of
  generated AssemblyScript.
- Keep the two ownership boundaries separate: versioned driver catalogs
  in `catalogs/` describe what a vendor serves; `policy.yaml` maps
  logical tier x role constants to catalog entries.
- Address catalog entries as `driverId:modelId`. Require
  `catalogVersion`; reject duplicate or unknown identifiers at load
  time.
- Reject unknown, disabled, incompatible, and over-budget models before
  performing network I/O, including when forced by environment
  variable.
- The builder sub-DSL emits a plain specification record and performs no
  I/O; `resolveSpec()` maps that record onto a catalog entry with an
  exhaustive switch, treating zero matches and ambiguous matches as hard
  configuration errors.
- Never log secrets, authorization headers, or complete provider error
  bodies.
- Keep policy resolution independent from wire-protocol adapters.
- Prefer deterministic, capped, sequential live smoke tests.
- Live tests are opt-in. Unit tests must mock network access.
- Ollama is the free local driver: optional auth, and the smoke reports
  `SKIP` on connection refused rather than failing.
- Add providers incrementally; deferred providers remain documented
  rather than partially implemented.

## Verification

- Run `npm test` in `eval-harness` after provider changes.
- Run the existing grader self-check after grader or prompt changes.
- Run live smoke tests only with explicit provider selection.
- Before declaring a task complete: `npm install` (runs the `prepare`
  script that installs `../sdk`), `npm test`, and a real
  `promptfoo eval` in `eval-harness`; and `just fmt` / `cargo test` /
  `just fix` from `codex-rs/` if Rust code changed.