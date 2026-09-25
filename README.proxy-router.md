# codex-proxy-router(1)

## NAME

`codex-proxy-router` — compiled-in routing proxy for the secure translating proxies

## SYNOPSIS

```shell
codex-proxy-router [--port PORT] [--server-info FILE] [--http-shutdown] [--secret-mode MODE]
```

## DESCRIPTION

`codex-proxy-router` is the single entry point for hosts that want all of the
secure translating proxies behind one loopback address.  It listens on
`127.0.0.1`, and routes each request by the first path segment — the upstream
host name — to the matching translating proxy, which it boots as a child
process:

```shell
codex-proxy-router (127.0.0.1:9090)
        │
        ├─ /opencode.ai/... ──▶ codex-opencode-proxy        (boots as child)
        └─ /mistral.ai/...  ──▶ codex-mistral-proxy (boots as child)
```

The remainder of the path is passed through untouched: a request to
`POST /opencode.ai/v1/responses` is forwarded as `POST /v1/responses` to the
`codex-opencode-proxy` child; `GET /mistral.ai/v1/models?client_version=X` is forwarded
as `GET /v1/models?client_version=X`.  Query strings, headers, and request
bodies are relayed verbatim; response bytes — including SSE streams — are
relayed straight back without buffering or parsing.  Adding a future proxy is
one entry in the router's compiled backend registry.

Both children serve their own contracts unchanged (see `README.opencode-proxy.md` and
`codex-rs/mistral-proxy/README.md`), and clients may still boot either proxy alone;
the router is only a dispatcher.

## OPTIONS

`--port PORT`
: TCP port to listen on (default: ephemeral; write `--server-info` to
  discover it).

`--server-info FILE`
: Write `{"port": N, "pid": N}` to FILE once the listener is ready.

`--http-shutdown`
: Enable a `GET /shutdown` endpoint that causes the process — and its child
  proxies — to exit cleanly.

`--secret-mode MODE`
: Secret supply channel to negotiate with child proxies (`secret-push`,
  `workload-identity`, or `env-debug`; default: `secret-push`). `env-debug`
  copies API keys into child environments and is for local testing only.
  See `docs/proxy-protocol.md`.

## ENDPOINTS SERVED

`POST|GET /opencode.ai/<contract>`
: Forwarded to the `codex-opencode-proxy` child with the `/opencode.ai` prefix
  removed.

`POST|GET /mistral.ai/<contract>`
: Forwarded to the `codex-mistral-proxy` child with the `/mistral.ai` prefix
  removed.

`GET /health`
: Router status and the compiled backend set. Child liveness is not probed;
  a request routed to a dead child surfaces the child's error.

`GET /shutdown`
: Only when `--http-shutdown` is set.

Anything else returns 404.

## SECURITY & SECRET SUPPLY MODES

The router and provider proxies implement the Prompt Cult Proxy Protocol v1
(see `docs/proxy-protocol.md`). Secret delivery is governed by `--secret-mode`:

### 1. `secret-push` (Production Mode)
In `secret-push` mode, the router and proxies keep secrets strictly out of
process environment tables. Child proxies are spawned keyless with a clean
environment and closed stdin. The router generates a 256-bit boot token from
the operating system's cryptographically secure random source, writes it to a
0600 file inside the child's temp directory, and passes the file path via
`--boot-token-file` — the token never appears on `argv`, where `ps` would
expose it. The router reads the provider keys from its own secure vault or
configuration and pushes them directly to each child's
`POST /protocol/v1/secrets` endpoint with the matching `X-Router-Boot-Token`.
The child deletes the token file at startup, ingests the key into `mlock(2)`-
protected memory, zeroizes the request body, consumes the single-use token,
and returns HTTP 204. Upstream requests are held (503) until key delivery
completes. The router never forwards `/protocol/v1/secrets` from its own
clients: the protocol control plane is child-local.

### 2. `workload-identity` (Cloud & Container Platforms)
In `workload-identity` mode, zero secret material is passed by the router.
Child proxies resolve credentials independently using platform mechanisms (KMS,
AWS IAM Roles for Service Accounts, Kubernetes Pod Identity, or HashiCorp Vault).
The router sanitizes the environment to preserve necessary system variables
(`PATH`, `HOME`, `CODEX_CONFIG_DIR`, SSL CA certificates, cloud tokens) while
filtering out API key secrets. The first-party proxies do **not** implement
this channel today — they never advertise it, and a router started with
`--secret-mode workload-identity` fails negotiation loudly against them. The
mode exists for commercial proxies that advertise `["workload-identity"]`.

### 3. `env-debug` (Local Testing Only)
In `env-debug` mode, the router copies allow-listed environment variable names
(`OPENCODE_API_KEY` for `codex-opencode-proxy`, `MISTRAL_API_KEY` for
`codex-mistral-proxy`) opaquely into the child's environment. The router
prints a prominent security warning to stderr. Child proxies immediately
unset the environment variable after reading and zeroize raw heap buffers.
This mode is strictly for local developer testing.

## SECURITY NOTES

- Binds to `127.0.0.1` only; it is not reachable from the network.
- Under the router, child proxies are spawned with `stdin=null`. Interactive
  stdin key entry is supported only when running a proxy standalone.
- When the dispatcher exits (normally or via `GET /shutdown`), the child
  proxies it started are cleanly terminated and reaped.
