# Dummy council-form fixture

This deterministic CRM-shaped site stands in for the legacy council forms the
browser extension will eventually operate.

It provides:

- unstable-looking browser interactions without an external dependency;
- a server-side action log as the test oracle;
- resettable contact data;
- a browser page that loads the packaged ThreadBox WASM through the same `.mjs`
  runtime used by the extension.

It does not execute a DSL and it does not call a model.

## Run

From `threadbox/`:

```sh
npm run build
./harness/dummy-form/start.sh 3456
```

Then open:

- `http://127.0.0.1:3456/` for the CRM fixture;
- `http://127.0.0.1:3456/runtime-lab.html` for the browser-WASM runtime lab.

Use `./harness/dummy-form/stop.sh` to stop it.
