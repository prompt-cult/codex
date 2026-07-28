# ThreadBox browser runtime

ThreadBox is a pre-alpha experiment in running an AssemblyScript-compiled
WebAssembly core inside a browser extension.

The immediate product problem is repetitive data entry into pathological legacy
grant-application forms. The eventual system will combine a small, task-specific
agent DSL with browser-loaded `.mjs` tools, content-script bridges, screenshots,
GUI localization, and deterministic form operations.

That DSL does not exist in this checkpoint. It will be designed from the real
grant-entry workflow rather than inherited from the removed general-purpose
agent-graph experiments.

## What exists

```text
AssemblyScript source
        |
        v
browser-compatible WASM
        |
        +--> Node tests (development and CI only)
        |
        v
extension/runtime.mjs
        |
        v
Manifest V3 service worker
```

- `guest/` contains the AssemblyScript compiler pin, one retained coordinate
  algebra module, and tests that instantiate its WASM with the standard Web
  `WebAssembly` API.
- `extension/` contains a minimal Manifest V3 extension runtime. It loads the
  same WASM artifact tested from Node.
- `harness/dummy-form/` is a deterministic legacy-CRM fixture with a server-side
  action log. It is a target for future browser-extension tests.
- `tools/package-extension.mjs` copies the compiled WASM into the unpacked
  extension directory.

## Commands

Run from `threadbox/`:

```sh
npm run check
npm run build
npm test
```

Each stage is independently callable and communicates through files and exit
status. The browser extension never depends on the Rust CLI runtime removed at
the pivot.

## Deliberately absent

- No general-purpose agent graph.
- No stable DSL or IR.
- No CLI execution runtime.
- No provider policy or model catalogue.
- No production content script.
- No credentials or live model proxy.
- No claim that the grant-entry workflow is automated yet.

The next specification should describe the council-form workflow, its browser
capabilities, and its failure semantics before adding a new DSL.
