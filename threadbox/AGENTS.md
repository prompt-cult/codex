# ThreadBox Agent Guidance

## Scope

This subtree owns the ThreadBox DSL, evaluation harness, provider
policy, and future runtime. It does not extend Codex provider routing.

## Provider policy

- Keep provider URLs, credentials, and concrete model selection out of
  generated AssemblyScript.
- Resolve logical roles through the curated `models.yaml` allowlist.
- Reject unknown, disabled, incompatible, and disallowed-cost models
  before performing network I/O.
- Never log secrets, authorization headers, or complete provider error
  bodies.
- Keep policy resolution independent from wire-protocol adapters.
- Prefer deterministic, capped, sequential live smoke tests.
- Live tests are opt-in. Unit tests must mock network access.
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