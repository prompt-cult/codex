# Prompt Cult Proxy Protocol v1

## 1. Overview & Purpose

The Prompt Cult Proxy Protocol standardizes the operational and security contract between provider proxies (such as `codex-mistral-proxy`, `codex-opencode-proxy`, and third-party/commercial implementations) and orchestration layers like `codex-proxy-router` or container sidecar managers.

**Core Invariant:** The purpose of provider proxies is to **keep secrets hidden**. Environment variables are strictly for local testing (`env-debug` mode). In secure production deployments, proxies obtain key material via authenticated direct injection (`secret-push`) or resolve credentials independently via platform identity mechanisms (`workload-identity`).

---

## 2. Proxy Identity & Configuration

Every conforming proxy implementation provides:
1. **Stable Identifier (`proxy_kind`)**: A unique string identifying the upstream provider and protocol dialect (e.g. `proxy-mistral-ai`, `proxy-opencode-zen`, `proxy-opencode-go`).
2. **Settings File**: Optional JSONC file loaded from `<codex_config_dir>/<proxy_kind>.jsonc` configuring upstream base URL overrides, log level, and model exclusion globs.
3. **Loopback HTTP Server**: Binds exclusively to `127.0.0.1`.
4. **Lifecycle Endpoints**:
   - `GET /health`: Returns JSON `{"status": "ok", "proxy": "<proxy_kind>", ...}`.
   - `GET /shutdown`: Clean process termination when started with `--http-shutdown`.

---

## 3. Protocol v1 Handshake (`server-info`)

When started with `--server-info <FILE>`, the proxy writes a single-line JSON descriptor once its listener is bound and ready to receive traffic:

```json
{
  "server_info_version": 1,
  "port": 41123,
  "pid": 5678,
  "proxy_kind": "proxy-opencode-zen",
  "protocol_version": 1,
  "secret_channels": ["env-debug", "secret-push"]
}
```

### Schema Fields
- `server_info_version` (*integer*, required): Schema version of the server info file itself (currently `1`).
- `port` (*integer*, required): TCP port bound on `127.0.0.1`.
- `pid` (*integer*, required): Process ID of the running proxy.
- `proxy_kind` (*string*, required): Conforming proxy identifier string.
- `protocol_version` (*integer*, required): Supported Proxy Protocol major version (must be `1`).
- `secret_channels` (*array of strings*, required): Capability advertisement listing the secret supply channels this proxy executable **actually implements**. A proxy must never advertise a channel it does not implement; conforming routers fail negotiation loudly against a false advertisement.
  - First-party proxies (`codex-mistral-proxy`, `codex-opencode-proxy`) implement `["env-debug", "secret-push"]`.
  - `workload-identity` is reserved for commercial proxies that resolve their own credentials (see section 4.3).

---

## 4. Secret Supply Channels

Protocol v1 defines three distinct secret supply channels:

### 4.1. `env-debug` (Local Testing Only)
- **Use Case:** Local developer testing and quick iterations.
- **Mechanism:** The caller sets allow-listed environment variables (e.g. `OPENCODE_API_KEY`, `MISTRAL_API_KEY`) on the child process.
- **Hardening Requirement:**
  1. The child proxy reads the variable, trims it, locks the value in memory (`mlock(2)`), and **immediately unsets the variable** from its process environment via `std::env::remove_var`.
  2. The raw heap-allocated string buffer is zeroized (`zeroize`) after constructing the header.
  3. The router emits a prominent security warning to `stderr` whenever running in `env-debug` mode indicating that secrets were passed via environment variables.

### 4.2. `secret-push` (Production Key Delivery)
- **Use Case:** Production environments where an orchestrator or router holds the API keys (from encrypted vaults, KMS, or CLI) and injects them into proxies without exposing them in process environment tables (`/proc/<pid>/environ`, `ps -E`, core dumps).
- **Mechanism:**
  1. The router generates a 256-bit boot token from the operating system's
     cryptographically secure random source, writes it to a `0600` boot-token
     file inside the child's server-info temp directory, and spawns the child
     with `--secret-channel secret-push --boot-token-file <FILE>`. The token
     never appears in `argv` (where `ps` exposes it) and no secret environment
     variables are provided.
  2. The child reads the boot-token file at startup, zeroizes and deletes the
     file immediately, and holds the token only in memory. Prior to key
     provisioning, the proxy starts its HTTP server. Any incoming upstream
     forwarding requests (e.g. `POST /v1/responses`, `GET /v1/models`) respond
     with HTTP `503 Service Unavailable` and error code `proxy_secret_pending`.
  3. The orchestrator issues an HTTP request to provision the secret:
     ```http
     POST /protocol/v1/secrets HTTP/1.1
     Host: 127.0.0.1:<port>
     Content-Type: application/json
     X-Router-Boot-Token: <TOKEN>

     {
       "channel": "secret-push",
       "key_id": "primary",
       "material": "<API_KEY>"
     }
     ```
  4. The child validates the `X-Router-Boot-Token`. On mismatch, it responds with `401 Unauthorized`.
  5. The child validates that `payload.channel == "secret-push"`; any other channel responds with `400 Bad Request` (`proxy_secret_channel_mismatch`).
  6. On valid token, the child trims the `material`, ingests the key into `mlock(2)`-protected memory, zeroizes the raw request body and the untrimmed payload copy, and returns `204 No Content`. Provisioning and replay rejection are a single atomic step, so concurrent pushes cannot double-provision.
  7. The boot token is single-use: the proxy consumes (invalidates) it on successful provisioning. Subsequent attempts to call `POST /protocol/v1/secrets` are rejected with `409 Conflict` (`proxy_secret_already_provisioned`).
  8. The proxy immediately transitions to fully ready, unlocking upstream request forwarding.

- **Note (state oracle):** After provisioning, a caller without a valid token
  can distinguish provisioned (`409`) from not-provisioned (`401`) because the
  consumed token short-circuits to `409` before authentication. The invariant
  holds — the token is consumed only after successful provisioning — and the
  endpoint binds loopback only, so this oracle is documented and accepted.

### 4.3. `workload-identity` (Production Cloud & Container Platforms)
- **Use Case:** Cloud environments (Kubernetes Pod Identity, AWS IAM Roles for Service Accounts / IRSA, GCP Workload Identity, HashiCorp Vault Agent).
- **Advertisement rule:** A proxy must not advertise `workload-identity` unless it fully implements credential resolution. The first-party proxies do **not** implement this channel today; they reject `--secret-channel workload-identity` at startup and advertise only `["env-debug", "secret-push"]`. A router asked for `--secret-mode workload-identity` against first-party children fails negotiation loudly. Commercial proxies that resolve their own credentials advertise `["workload-identity"]` and nothing else.
- **Mechanism:**
  1. The proxy is spawned with `--secret-channel workload-identity`.
  2. Zero secret material is passed by the router or orchestrator.
  3. The orchestrator preserves non-secret system environment variables required by workload identity SDKs (e.g. `PATH`, `HOME`, `SSL_CERT_FILE`, `SSL_CERT_DIR`, `KUBERNETES_SERVICE_HOST`, `AWS_REGION`, `AWS_WEB_IDENTITY_TOKEN_FILE`, `VAULT_ADDR`) while stripping all provider API keys.
  4. The proxy resolves its own credentials via provider-specific identity exchanges or local volume-mounted tokens.

---

## 5. Router Negotiation State Machine

When `codex-proxy-router` initializes child backends:

```
[Start Router]
      │
      ▼
[Parse --secret-mode {env-debug | secret-push | workload-identity}]
      │
      ▼
[Spawn child with appropriate env filter and flags]
      │
      ▼
[Wait for server-info JSON]
      │
      ▼
[Validate protocol_version == 1]
      │
   ├── (Mismatch) ──▶ [Terminate child, Abort with ProtocolVersionMismatch]
   │
[Match child secret_channels against requested --secret-mode]
      │
   ├── (No match) ──▶ [Terminate child, Abort with IncompatibleSecretChannel]
   │
[Execute Channel Provisioning]
   ├── env-debug: Emit stderr warning banner, child unsets env, ready immediately
   ├── secret-push: POST /protocol/v1/secrets with boot token, verify 204, ready
   └── workload-identity: Verify child readiness, ready
```

---

## 6. Versioning Rules

- `protocol_version` is an integer tracking major breaking changes in wire schemas or lifecycle contracts.
- Any breaking change to the `server-info` shape, `/protocol/v1/secrets` endpoint, or lifecycle transitions requires incrementing `protocol_version` to `2`.
- Adding new `secret_channels` or non-required fields to request/response bodies is minor and backwards-compatible. Conforming routers ignore unknown `secret_channels` entries that are not requested.
