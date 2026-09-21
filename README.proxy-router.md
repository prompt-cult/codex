# codex-proxy-router(1)

## NAME

`codex-proxy-router` — compiled-in routing proxy for the secure translating proxies

## SYNOPSIS

```shell
codex-proxy-router [--port PORT] [--server-info FILE] [--http-shutdown]
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

Both children serve their own contracts unchanged (see `README.zen-proxy.md` and
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

## SECURITY

The router holds no key material and never sees any.  Each child proxy owns
its own hardened key state: the key is resolved by the proxy itself into
`mlock(2)`-protected memory (stdin- or environment-vaulted today) and
injected into upstream requests inside that process.  Keys never move through
the dispatcher.

At spawn the router sanitizes the child environment to an **allow-list of
environment variable names** (`OPENCODE_API_KEY` for `codex-opencode-proxy`,
`MISTRAL_API_KEY` for `codex-mistral-proxy`).  Names on the list that exist
in the router's own environment are copied opaquely into the child's
environment; every other name is dropped.  Values are never inspected,
logged, or stored.

This allow-list is a transitional affordance.  The end state is that nothing
is passed at all: children will resolve their secrets themselves through
workload identities — a KMS, a Kubernetes volume mount, or another hardened
key store as yet undefined — each from a different vault, one vault per proxy.

## SECURITY NOTES

- Binds to `127.0.0.1` only; it is not reachable from the network.
- Child proxies are booted with stdin closed, so a child that cannot resolve
  its key from its vault fails fast at startup with a clear error instead of
  blocking.
- When the dispatcher exits (normally or via `GET /shutdown`), the child
  proxies it started are terminated.
