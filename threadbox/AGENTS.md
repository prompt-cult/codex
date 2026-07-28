# ThreadBox browser-runtime guidance

## Scope

ThreadBox is currently a browser-extension WebAssembly runtime chassis. The
grant-entry DSL has intentionally not been specified yet.

## Boundaries

- Production execution happens through the browser WebAssembly API.
- Node and command-line programs may build and test artifacts, but must not
  become a second production runtime.
- Browser tools are ES modules with narrow inputs and outputs.
- The extension loads only packaged, local code. Do not add remote scripts.
- Keep credentials, provider URLs, and customer data out of generated source and
  committed fixtures.
- Do not add a general-purpose graph abstraction before the grant-entry DSL is
  specified.

## Unix composition

- Build stages communicate through files, stdout/stderr, and exit status.
- Each script does one job and is independently testable.
- Keep AssemblyScript compilation separate from extension packaging.
- Tests instantiate the same WASM bytes packaged into the extension.

## Verification

Run from `threadbox/`:

```sh
npm run check
npm run build
npm test
```

Live model calls and real council sites are never part of the default test gate.
