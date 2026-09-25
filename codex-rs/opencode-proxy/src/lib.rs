//! `codex-opencode-proxy` — A translating proxy for OpenCode Zen and Go.
//!
//! Accepts OAI Responses API requests from codex-rs and routes them to the
//! appropriate upstream endpoint based on model family:
//!
//! - **GPT models** (`gpt-*`, `o1*`, `o3*`, `grok-*`, `muse-*`) → passthrough
//!   to `{upstream}/responses`
//! - **Claude models** (`claude-*`, `minimax-*`, `qwen*`) → translate to the
//!   Anthropic Messages API at `{upstream}/messages`
//! - **Chat models** (`glm-*`, `kimi-*`, `deepseek-*`, `longcat-*`,
//!   `mimo-*`, `hy-*`) → translate to the OpenAI-compatible Chat Completions
//!   API at `{upstream}/chat/completions`
//!
//! One key (`OPENCODE_API_KEY`, env-first with stdin fallback) serves both
//! endpoints: the paid Zen base (`https://opencode.ai/zen/v1`) and the open
//! Go base (`https://opencode.ai/zen/go/v1`). Which endpoint is in use is
//! decided by the resolved upstream base path: a `/go` path selects
//! [`ProxyKind::OpencodeGo`] (log prefix, health JSON and the
//! `proxy-opencode-go.jsonc` config file), anything else selects
//! [`ProxyKind::OpencodeZen`].
//!
//! The Go endpoint requires a non-generic `User-Agent` and a stable
//! `x-opencode-session` header per conversation; this proxy is the client, so
//! it sends `codex-opencode-proxy/<version>` and one UUIDv4 generated at
//! startup on every upstream request (harmless for Zen).

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::net::SocketAddr;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use clap::Parser;
use codex_proxy_protocol::LogLevel;
use codex_proxy_protocol::ProxyDefaults;
use codex_proxy_protocol::ProxyKind;
use codex_proxy_protocol::load_config;
use codex_proxy_protocol::protocol::SecretState;
use globset::GlobSet;
use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HOST;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use tiny_http::Header;
use tiny_http::Method;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;
use tiny_http::StatusCode;
use uuid::Uuid;
use zeroize::Zeroize;

mod anthropic_translate_request;
mod anthropic_translate_sse;
mod chat_models_translate;
mod chat_translate_request;
mod chat_translate_sse;
mod read_api_key;
mod routing;

use read_api_key::read_auth_header;
use routing::ModelFamily;

/// Compiled-in User-Agent; the Go endpoint rejects generic SDK names.
const USER_AGENT: &str = concat!("codex-opencode-proxy/", env!("CARGO_PKG_VERSION"));

/// Compiled-in defaults used when no `proxy-opencode-{zen,go}.jsonc` exists.
/// Both endpoints curate their own model lists, so there is no default
/// exclusion glob.
const OPENCODE_DEFAULTS: ProxyDefaults = ProxyDefaults {
    upstream_base_url: DEFAULT_UPSTREAM_BASE,
    model_exclude_globs: &[],
};

const DEFAULT_UPSTREAM_BASE: &str = "https://opencode.ai/zen/v1";

/// Select the proxy kind from an upstream base URL: the Go endpoint is
/// recognized by a `/go` path segment (e.g. `/zen/go/v1`).
fn kind_for_base(base: &str) -> ProxyKind {
    let path = Url::parse(base)
        .map(|url| url.path().to_string())
        .unwrap_or_default();
    if path.contains("/go") {
        ProxyKind::OpencodeGo
    } else {
        ProxyKind::OpencodeZen
    }
}

/// Classification of an incoming request path, ignoring any query string.
///
/// Codex always appends `?client_version=X` to `/v1/models`, so route matching
/// must compare the path only. Matching the raw URL (path + query) is the bug
/// that made model discovery 403.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RouteKind {
    Models,
    Responses,
    Shutdown,
    Health,
    Forbidden,
}

/// Map a raw request URL (path plus optional query) to a [`RouteKind`].
fn classify_route(raw_url: &str) -> RouteKind {
    let path = raw_url.split('?').next().unwrap_or(raw_url);
    match path {
        "/v1/models" | "/models" => RouteKind::Models,
        "/v1/responses" => RouteKind::Responses,
        "/shutdown" => RouteKind::Shutdown,
        "/health" => RouteKind::Health,
        _ => RouteKind::Forbidden,
    }
}

/// CLI arguments for the OpenCode translating proxy.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "codex-opencode-proxy",
    about = "Translating proxy: OAI Responses API ↔ OpenCode Zen/Go (GPT passthrough + Anthropic + Chat translation)"
)]
pub struct Args {
    /// Port to listen on. If not set, an ephemeral port is used.
    #[arg(long)]
    pub port: Option<u16>,

    /// Path to a JSON file to write startup info (single line). Includes {"port": <u16>}.
    #[arg(long, value_name = "FILE")]
    pub server_info: Option<PathBuf>,

    /// Enable HTTP shutdown endpoint at GET /shutdown.
    #[arg(long)]
    pub http_shutdown: bool,

    /// Base URL of the OpenCode API. Overrides `upstream_base_url` in
    /// `proxy-opencode-zen.jsonc` / `proxy-opencode-go.jsonc`; compiled-in
    /// default: https://opencode.ai/zen/v1. A `/go` path selects the Go kind.
    /// Responses passthrough goes to `{base}/responses`, Anthropic translation
    /// to `{base}/messages`, Chat translation to `{base}/chat/completions`,
    /// model listings to `{base}/models`.
    #[arg(long)]
    pub upstream_base: Option<String>,

    /// Secret supply channel (protocol v1). `env-debug` reads the key from
    /// the environment or stdin at startup; `secret-push` starts keyless and
    /// waits for the authenticated `POST /protocol/v1/secrets` delivery.
    /// `workload-identity` is not implemented by this proxy and is rejected
    /// at startup (it exists for commercial proxies).
    #[arg(long, value_name = "CHANNEL", default_value = "env-debug")]
    pub secret_channel: String,

    /// Path to the file holding the ephemeral single-use boot token for
    /// `--secret-channel secret-push` (protocol v1 section 4.2). The token
    /// must not be passed on argv (`ps` exposes it); the proxy zeroizes and
    /// deletes the file at startup. Required in secret-push mode.
    #[arg(long, value_name = "FILE")]
    pub boot_token_file: Option<PathBuf>,
}

/// Secret supply channels this proxy executable implements (protocol v1).
/// `workload-identity` is deliberately absent: this proxy does not resolve
/// its own credentials, and a false advertisement would defeat negotiation.
const SECRET_CHANNELS: &[codex_proxy_protocol::protocol::SecretChannel] = &[
    codex_proxy_protocol::protocol::SecretChannel::EnvDebug,
    codex_proxy_protocol::protocol::SecretChannel::SecretPush,
];

struct ProxyConfig {
    /// Which endpoint flavor is being served; drives the log prefix, the
    /// health JSON "proxy" field and the config filename.
    kind: ProxyKind,
    /// Base URL (no trailing slash), e.g. "https://opencode.ai/zen/v1"
    upstream_base: String,
    /// Pre-parsed host header value for upstream requests
    host_header: HeaderValue,
    /// Glob exclusions applied to discovered model IDs.
    exclude: GlobSet,
    /// Per-model metadata overrides from the proxy config file.
    model_overrides: HashMap<String, chat_models_translate::ModelTranslateOverride>,
    /// Logging verbosity from the proxy config file.
    log_level: LogLevel,
    /// Per-process request counter for verbose routing logs.
    request_counter: AtomicU64,
    /// User-Agent header sent on every upstream request.
    user_agent: HeaderValue,
    /// Stable per-conversation session header generated once at startup.
    session_header: HeaderValue,
}

/// Entry point.
pub fn run_main(args: Args) -> Result<()> {
    let secret_channel: codex_proxy_protocol::protocol::SecretChannel =
        serde_json::from_value(serde_json::Value::String(args.secret_channel.clone()))
            .context("parsing --secret-channel")?;
    let secret_state = match secret_channel {
        codex_proxy_protocol::protocol::SecretChannel::SecretPush => {
            let path = args
                .boot_token_file
                .as_ref()
                .context("--secret-channel secret-push requires --boot-token-file")?;
            SecretState::pending(codex_proxy_protocol::protocol::read_boot_token_file(path)?)
        }
        codex_proxy_protocol::protocol::SecretChannel::EnvDebug => {
            SecretState::provisioned_at_startup(read_auth_header()?)
        }
        codex_proxy_protocol::protocol::SecretChannel::WorkloadIdentity => {
            anyhow::bail!(
                "workload-identity is not implemented by this proxy; it must not be advertised"
            )
        }
    };

    // CODEX_CONFIG_DIR takes precedence over CODEX_HOME inside
    // find_codex_home; the proxy config lives next to the TUI's config.toml.
    let config_dir = codex_utils_home_dir::find_codex_home()
        .context("resolving codex config dir for proxy config")?;

    // KIND is derived from the upstream base: CLI flag first; without it the
    // zen settings file is consulted and a `/go` base there switches to the
    // Go kind and its `proxy-opencode-go.jsonc` settings file.
    let (kind, resolved, config_upstream) = match args.upstream_base.as_deref() {
        Some(cli_base) => {
            let kind = kind_for_base(cli_base);
            let resolved = load_config(kind, config_dir.as_path(), &OPENCODE_DEFAULTS)?;
            (kind, resolved, None)
        }
        None => {
            let zen = load_config(
                ProxyKind::OpencodeZen,
                config_dir.as_path(),
                &OPENCODE_DEFAULTS,
            )?;
            let provisional = zen
                .upstream_base_url
                .clone()
                .unwrap_or_else(|| DEFAULT_UPSTREAM_BASE.to_string());
            if kind_for_base(&provisional) == ProxyKind::OpencodeGo {
                let go = load_config(
                    ProxyKind::OpencodeGo,
                    config_dir.as_path(),
                    &OPENCODE_DEFAULTS,
                )?;
                let base = go.upstream_base_url.clone().unwrap_or(provisional);
                (ProxyKind::OpencodeGo, go, Some(base))
            } else {
                let base = zen.upstream_base_url.clone();
                (ProxyKind::OpencodeZen, zen, base)
            }
        }
    };
    match &resolved.loaded_from {
        Some(path) => eprintln!("{}: loaded config from {}", kind.as_str(), path.display()),
        None => eprintln!(
            "{}: no {} found, using built-in defaults",
            kind.as_str(),
            kind.config_filename()
        ),
    }

    // Precedence: CLI flag > config file > compiled-in default.
    let upstream_base = args
        .upstream_base
        .or(config_upstream)
        .unwrap_or_else(|| DEFAULT_UPSTREAM_BASE.to_string());
    let upstream_base = upstream_base.trim_end_matches('/').to_string();
    let parsed = Url::parse(&upstream_base).context("parsing upstream base URL")?;
    let host = match (parsed.host_str(), parsed.port()) {
        (Some(h), Some(p)) => format!("{h}:{p}"),
        (Some(h), None) => h.to_string(),
        _ => return Err(anyhow!("upstream base URL must include a host")),
    };
    let host_header =
        HeaderValue::from_str(&host).context("constructing Host header from upstream URL")?;

    let model_overrides = resolved
        .model_overrides
        .iter()
        .map(|(id, ovr)| {
            (
                id.clone(),
                chat_models_translate::ModelTranslateOverride {
                    base_instructions: ovr.base_instructions.clone(),
                },
            )
        })
        .collect();

    let config = Arc::new(ProxyConfig {
        kind,
        upstream_base,
        host_header,
        exclude: resolved.exclude,
        model_overrides,
        log_level: resolved.log_level,
        request_counter: AtomicU64::new(0),
        user_agent: HeaderValue::from_static(USER_AGENT),
        session_header: HeaderValue::from_str(&Uuid::new_v4().to_string())
            .context("constructing session header")?,
    });

    let (listener, bound_addr) = bind_listener(args.port)?;
    if let Some(path) = args.server_info.as_ref() {
        write_server_info(path, config.kind, bound_addr.port())?;
    }
    let server = Server::from_listener(listener, None)
        .map_err(|err| anyhow!("creating HTTP server: {err}"))?;
    let client = Arc::new(
        Client::builder()
            .timeout(None::<Duration>)
            .build()
            .context("building reqwest client")?,
    );

    eprintln!(
        "{} listening on {bound_addr} → {}",
        config.kind.as_str(),
        config.upstream_base
    );

    let http_shutdown = args.http_shutdown;
    let secret_state = Arc::new(secret_state);
    for request in server.incoming_requests() {
        let client = client.clone();
        let config = config.clone();
        let secret_state = secret_state.clone();
        std::thread::spawn(move || {
            let method = request.method().clone();
            let route = classify_route(request.url());

            if http_shutdown && method == Method::Get && route == RouteKind::Shutdown {
                let _ = request.respond(Response::new_empty(StatusCode(200)));
                std::process::exit(0);
            }

            if method == Method::Get && route == RouteKind::Health {
                let body = serde_json::json!({
                    "status": "ok",
                    "proxy": config.kind.as_str(),
                    "upstream": config.upstream_base,
                });
                let data = serde_json::to_vec(&body).unwrap_or_default();
                let resp = Response::from_data(data)
                    .with_status_code(StatusCode(200))
                    .with_header(
                        Header::from_bytes(b"content-type", b"application/json")
                            .unwrap_or_else(|_| unreachable!()),
                    );
                let _ = request.respond(resp);
                return;
            }

            if let Err(e) = handle_request(&client, &secret_state, &config, request) {
                eprintln!("{} error: {e}", config.kind.as_str());
            }
        });
    }

    Err(anyhow!("server stopped unexpectedly"))
}

fn bind_listener(port: Option<u16>) -> Result<(TcpListener, SocketAddr)> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = TcpListener::bind(addr).with_context(|| format!("failed to bind {addr}"))?;
    let bound = listener.local_addr().context("failed to read local_addr")?;
    Ok((listener, bound))
}

fn write_server_info(path: &Path, kind: ProxyKind, port: u16) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let info = codex_proxy_protocol::protocol::ServerInfoV1 {
        server_info_version: 1,
        port,
        pid: std::process::id(),
        proxy_kind: kind.as_str().to_string(),
        protocol_version: codex_proxy_protocol::protocol::PROTOCOL_VERSION,
        secret_channels: codex_proxy_protocol::protocol::SecretChannelList(
            SECRET_CHANNELS.to_vec(),
        ),
    };
    info.write_to_file(path)?;
    Ok(())
}

fn handle_request(
    client: &Client,
    secret_state: &SecretState,
    config: &ProxyConfig,
    req: Request,
) -> Result<()> {
    let method = req.method().clone();
    let url = req.url().to_string();

    // Protocol v1 section 4.2: the authenticated direct memory push endpoint.
    // Handled in every state so post-provisioning replays get their 409.
    if method == Method::Post
        && url == codex_proxy_protocol::protocol::SECRETS_ENDPOINT
        && secret_state.secret_push_mode()
    {
        return handle_secret_push(secret_state, req);
    }

    // Upstream forwarding holds (503) until the key is provisioned.
    let Some(auth_header) = secret_state.get() else {
        eprintln!(
            "{}: 503 proxy_secret_pending for {method} {url}",
            config.kind.as_str()
        );
        let body = serde_json::json!({
            "error": {
                "message": "proxy has not been provisioned with its API key yet",
                "type": "proxy_secret_pending",
            }
        });
        let data = serde_json::to_vec(&body).unwrap_or_default();
        let resp = Response::from_data(data)
            .with_status_code(StatusCode(503))
            .with_header(
                Header::from_bytes(b"content-type", b"application/json")
                    .unwrap_or_else(|_| unreachable!()),
            );
        let _ = req.respond(resp);
        return Ok(());
    };

    let route = classify_route(&url);
    eprintln!("{}: {method} {url} -> {route:?}", config.kind.as_str());

    // GET /v1/models — translate the upstream model list into the codex
    // `ModelsResponse` shape for dynamic discovery by the main app.
    if method == Method::Get && route == RouteKind::Models {
        return handle_models_request(client, auth_header, config, req);
    }

    // POST /v1/responses — route by model family.
    if method == Method::Post && route == RouteKind::Responses {
        return handle_responses_request(client, auth_header, config, req);
    }

    eprintln!("{}: 403 forbidden for {method} {url}", config.kind.as_str());
    if let Err(e) = req.respond(Response::new_empty(StatusCode(403))) {
        eprintln!("{}: failed to respond 403: {e}", config.kind.as_str());
    }
    Ok(())
}

/// JSON error response for the secret-push control endpoint.
fn respond_secret_error(req: Request, status: u16, code: &str, message: &str) -> Result<()> {
    let body = serde_json::json!({
        "error": {
            "message": message,
            "type": code,
        }
    });
    let data = serde_json::to_vec(&body).unwrap_or_default();
    let resp = Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes(b"content-type", b"application/json")
                .unwrap_or_else(|_| unreachable!()),
        );
    let _ = req.respond(resp);
    Ok(())
}

/// Protocol v1 section 4.2: ingest the pushed key material into mlock(2)
/// memory. Authenticated by the ephemeral single-use boot token; the token is
/// consumed on success so a replay always lands on `409`.
fn handle_secret_push(secret_state: &SecretState, mut req: Request) -> Result<()> {
    let Some(expected_token) = secret_state.boot_token() else {
        // The token was consumed: the secret is provisioned, so this is a
        // replay, not an authentication failure. Note: this is an
        // unauthenticated provisioning-state oracle (409 vs 401); it is
        // documented in the protocol spec and acceptable on loopback-only
        // bindings because the token is consumed only after successful
        // provisioning.
        return respond_secret_error(
            req,
            409,
            "proxy_secret_already_provisioned",
            "secret already provisioned",
        );
    };
    let supplied =
        req.headers()
            .iter()
            // HTTP header names are case-insensitive; tiny_http preserves the
            // received casing, so compare case-insensitively.
            .find(|h| {
                h.field.as_str().as_bytes().eq_ignore_ascii_case(
                    codex_proxy_protocol::protocol::BOOT_TOKEN_HEADER.as_bytes(),
                )
            })
            .map(|h| h.value.as_str().to_string());
    let token_matches = supplied
        .as_deref()
        .map(|supplied| {
            constant_time_eq::constant_time_eq(supplied.as_bytes(), expected_token.as_bytes())
        })
        .unwrap_or(false);
    if !token_matches {
        return respond_secret_error(
            req,
            401,
            "proxy_secret_unauthorized",
            "invalid or missing boot token",
        );
    }

    if secret_state.get().is_some() {
        return respond_secret_error(
            req,
            409,
            "proxy_secret_already_provisioned",
            "secret already provisioned",
        );
    }

    let mut body_bytes = Vec::new();
    req.as_reader().read_to_end(&mut body_bytes)?;
    let parsed =
        serde_json::from_slice::<codex_proxy_protocol::protocol::SecretPushPayload>(&body_bytes);
    body_bytes.zeroize();
    let mut payload = match parsed {
        Ok(payload) => payload,
        Err(_) => {
            return respond_secret_error(
                req,
                400,
                "proxy_secret_invalid_payload",
                "secret-push payload is not valid protocol v1 JSON",
            );
        }
    };

    // Only the negotiated channel may be delivered on this endpoint.
    if payload.channel != codex_proxy_protocol::protocol::SecretChannel::SecretPush {
        payload.material.zeroize();
        return respond_secret_error(
            req,
            400,
            "proxy_secret_channel_mismatch",
            "payload channel does not match the secret-push endpoint",
        );
    }

    let mut material = payload.material.trim().to_string();
    payload.material.zeroize();
    let header = match read_api_key::auth_header_from_key(&material) {
        Ok(header) => header,
        Err(_) => {
            material.zeroize();
            return respond_secret_error(
                req,
                400,
                "proxy_secret_invalid_material",
                "secret material failed validation",
            );
        }
    };
    material.zeroize();

    if !secret_state.provision_once(header) {
        return respond_secret_error(
            req,
            409,
            "proxy_secret_already_provisioned",
            "secret already provisioned",
        );
    }
    secret_state.consume_boot_token();
    let _ = req.respond(Response::new_empty(StatusCode(204)));
    Ok(())
}

/// POST /v1/responses: classify the model family and dispatch to passthrough
/// or translation.
fn handle_responses_request(
    client: &Client,
    auth_header: &'static str,
    config: &ProxyConfig,
    mut req: Request,
) -> Result<()> {
    // Extract forwarding headers before consuming the request body.
    let fwd_headers = build_upstream_headers(auth_header, config, &req);

    // Read request body (consumes the reader — must happen after header
    // extraction).
    let mut body_bytes = Vec::new();
    req.as_reader().read_to_end(&mut body_bytes)?;

    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes).context("parsing request JSON")?;
    let model = body["model"].as_str().unwrap_or("").to_string();
    let is_stream = body["stream"].as_bool().unwrap_or(false);

    let family = routing::classify_model(&model);
    let verbose = config.log_level == LogLevel::Verbose;
    let req_id = config.request_counter.fetch_add(1, Ordering::Relaxed) + 1;
    let verbose_req = verbose.then_some(req_id);
    if verbose {
        eprintln!(
            "{}: req#{req_id} model={model} family={family:?} stream={is_stream}",
            config.kind.as_str()
        );
    } else {
        eprintln!(
            "{}: model={model} family={family:?} stream={is_stream}",
            config.kind.as_str()
        );
    }

    match family {
        ModelFamily::Gpt => handle_gpt_passthrough(client, config, req, fwd_headers, &body_bytes),
        ModelFamily::Claude => {
            handle_claude_translate(client, auth_header, config, req, &body, &model, is_stream)
        }
        ModelFamily::Chat => handle_chat_translate(
            client,
            auth_header,
            config,
            ChatRequest {
                req,
                body,
                model,
                is_stream,
                verbose_req,
            },
        ),
        ModelFamily::Unknown => {
            let err_body = serde_json::json!({
                "error": {
                    "message": format!("{}: no route for model '{model}'", config.kind.as_str()),
                    "type": "proxy_not_implemented",
                }
            });
            let data = serde_json::to_vec(&err_body).unwrap_or_default();
            let resp = Response::from_data(data)
                .with_status_code(StatusCode(501))
                .with_header(
                    Header::from_bytes(b"content-type", b"application/json")
                        .unwrap_or_else(|_| unreachable!()),
                );
            let _ = req.respond(resp);
            Ok(())
        }
    }
}

/// GET /v1/models: fetch `{upstream}/models` and translate the raw list
/// into the codex `ModelsResponse` shape so `codex-api` can deserialize it.
///
/// Only chat-capable models are returned, minus any ID matching the configured
/// exclusion globs. On any upstream error the original error response is
/// relayed unchanged so the app can log the real cause.
fn handle_models_request(
    client: &Client,
    auth_header: &'static str,
    config: &ProxyConfig,
    req: Request,
) -> Result<()> {
    let upstream_url = format!("{}/models", config.upstream_base);
    eprintln!("{}: fetching upstream {upstream_url}", config.kind.as_str());

    let mut headers = HeaderMap::new();
    apply_identity_headers(&mut headers, auth_header, config);

    let upstream_resp = client
        .get(&upstream_url)
        .headers(headers)
        .send()
        .context("forwarding models request to upstream")?;

    eprintln!(
        "{}: upstream responded {}",
        config.kind.as_str(),
        upstream_resp.status()
    );

    // Relay non-200 responses verbatim so the app can log the real cause.
    if upstream_resp.status().as_u16() != 200 {
        return relay_response(req, upstream_resp, config.kind);
    }

    let raw = upstream_resp
        .bytes()
        .context("reading upstream models response")?;
    let translated = chat_models_translate::translate_chat_models(
        &raw,
        &config.exclude,
        &config.model_overrides,
    )
    .context("translating upstream /models to ModelsResponse")?;
    let kept = translated.response.models.len();
    let data = serde_json::to_vec(&translated.response).context("serializing ModelsResponse")?;

    eprintln!(
        "{}: loaded {} models, {} after exclusions",
        config.kind.as_str(),
        translated.chat_loaded,
        kept
    );

    let resp = Response::from_data(data)
        .with_status_code(StatusCode(200))
        .with_header(
            Header::from_bytes(b"content-type", b"application/json")
                .unwrap_or_else(|_| unreachable!()),
        );
    if let Err(e) = req.respond(resp) {
        eprintln!("{}: failed to respond models: {e}", config.kind.as_str());
    }
    Ok(())
}

/// GPT: passthrough to `{upstream}/responses` with the original body bytes.
fn handle_gpt_passthrough(
    client: &Client,
    config: &ProxyConfig,
    req: Request,
    headers: HeaderMap,
    body_bytes: &[u8],
) -> Result<()> {
    let upstream_url = format!("{}/responses", config.upstream_base);

    let upstream_resp = client
        .post(&upstream_url)
        .headers(headers)
        .body(body_bytes.to_vec())
        .send()
        .context("forwarding GPT request to upstream")?;

    relay_response(req, upstream_resp, config.kind)
}

/// Claude: translate OAI Responses → Anthropic Messages, then translate back.
fn handle_claude_translate(
    client: &Client,
    auth_header: &'static str,
    config: &ProxyConfig,
    req: Request,
    body: &serde_json::Value,
    model: &str,
    is_stream: bool,
) -> Result<()> {
    let upstream_url = format!("{}/messages", config.upstream_base);
    let anthropic_body = anthropic_translate_request::oai_to_anthropic(body);

    // Claude uses x-api-key rather than Bearer for the Anthropic format.
    // Extract just the key from "Bearer <key>".
    let api_key = auth_header.strip_prefix("Bearer ").unwrap_or(auth_header);

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-api-key"),
        HeaderValue::from_str(api_key).unwrap_or_else(|_| HeaderValue::from_static("")),
    );
    headers.insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("application/json"),
    );
    headers.insert(
        HeaderName::from_static("anthropic-version"),
        HeaderValue::from_static("2023-06-01"),
    );
    headers.insert(HOST, config.host_header.clone());
    apply_identity_headers(&mut headers, auth_header, config);
    if is_stream {
        headers.insert(
            HeaderName::from_static("accept"),
            HeaderValue::from_static("text/event-stream"),
        );
    }

    let upstream_resp = client
        .post(&upstream_url)
        .headers(headers)
        .json(&anthropic_body)
        .send()
        .context("forwarding Claude request to upstream")?;

    if upstream_resp.status().as_u16() != 200 {
        return relay_response(req, upstream_resp, config.kind);
    }

    if !is_stream {
        // Non-streaming: translate the single JSON response.
        let ant_body: serde_json::Value =
            upstream_resp.json().context("reading Claude response")?;
        let oai_resp = anthropic_translate_request::anthropic_response_to_oai(&ant_body, model);
        let data = serde_json::to_vec(&oai_resp).unwrap_or_default();
        let resp = Response::from_data(data)
            .with_status_code(StatusCode(200))
            .with_header(
                Header::from_bytes(b"content-type", b"application/json")
                    .unwrap_or_else(|_| unreachable!()),
            );
        let _ = req.respond(resp);
        return Ok(());
    }

    // Streaming: translate Anthropic SSE → OAI Responses SSE.
    let translator = anthropic_translate_sse::AnthropicToOaiStream::new(
        model.to_string(),
        upstream_resp,
        config.kind.as_str(),
    );
    let resp = Response::new(
        StatusCode(200),
        vec![
            Header::from_bytes(b"content-type", b"text/event-stream")
                .unwrap_or_else(|_| unreachable!()),
            Header::from_bytes(b"cache-control", b"no-cache").unwrap_or_else(|_| unreachable!()),
            Header::from_bytes(b"x-accel-buffering", b"no").unwrap_or_else(|_| unreachable!()),
        ],
        translator,
        None,
        None,
    );
    let _ = req.respond(resp);
    Ok(())
}

/// A parsed Chat-family request routed to Chat Completions translation.
struct ChatRequest {
    req: Request,
    body: serde_json::Value,
    model: String,
    is_stream: bool,
    verbose_req: Option<u64>,
}

/// Chat: translate OAI Responses → Chat Completions, then translate back.
fn handle_chat_translate(
    client: &Client,
    auth_header: &'static str,
    config: &ProxyConfig,
    chat_req: ChatRequest,
) -> Result<()> {
    let ChatRequest {
        req,
        body,
        model,
        is_stream,
        verbose_req,
    } = chat_req;
    let fwd_headers = build_upstream_headers(auth_header, config, &req);
    let chat_body = chat_translate_request::oai_to_chat(&body);
    let upstream_url = format!("{}/chat/completions", config.upstream_base);

    let upstream_resp = client
        .post(&upstream_url)
        .headers(fwd_headers)
        .json(&chat_body)
        .send()
        .context("forwarding chat request to upstream")?;

    if upstream_resp.status().as_u16() != 200 {
        return relay_response(req, upstream_resp, config.kind);
    }

    if !is_stream {
        let chat_json: serde_json::Value = upstream_resp.json().context("reading chat response")?;
        if verbose_req.is_some() {
            let upstream_model = chat_json["model"].as_str().unwrap_or("");
            eprintln!(
                "{}: req#{} upstream_model={upstream_model}",
                config.kind.as_str(),
                verbose_req.unwrap_or(0)
            );
        }
        let oai_resp = chat_translate_request::chat_response_to_oai(&chat_json, &model);
        let data = serde_json::to_vec(&oai_resp).unwrap_or_default();
        let resp = Response::from_data(data)
            .with_status_code(StatusCode(200))
            .with_header(
                Header::from_bytes(b"content-type", b"application/json")
                    .unwrap_or_else(|_| unreachable!()),
            );
        if let Err(e) = req.respond(resp) {
            eprintln!("{}: failed to respond chat: {e}", config.kind.as_str());
        }
        return Ok(());
    }

    // Streaming: translate Chat Completions SSE → OAI Responses SSE.
    let translator = chat_translate_sse::ChatToOaiStream::new(
        model,
        Box::new(upstream_resp),
        verbose_req,
        config.kind.as_str(),
    );
    let resp = Response::new(
        StatusCode(200),
        vec![
            Header::from_bytes(b"content-type", b"text/event-stream")
                .unwrap_or_else(|_| unreachable!()),
            Header::from_bytes(b"cache-control", b"no-cache").unwrap_or_else(|_| unreachable!()),
            Header::from_bytes(b"x-accel-buffering", b"no").unwrap_or_else(|_| unreachable!()),
        ],
        translator,
        None,
        None,
    );
    if let Err(e) = req.respond(resp) {
        eprintln!("{}: failed to respond stream: {e}", config.kind.as_str());
    }
    Ok(())
}

/// Inject the auth, host, and proxy-identity headers (User-Agent and
/// x-opencode-session) required on every upstream request. Client-supplied
/// values of the identity headers are never forwarded — this proxy is the
/// client.
fn apply_identity_headers(
    headers: &mut HeaderMap,
    auth_header: &'static str,
    config: &ProxyConfig,
) {
    let mut auth_value = HeaderValue::from_static(auth_header);
    auth_value.set_sensitive(true);
    headers.insert(AUTHORIZATION, auth_value);
    headers.insert(HOST, config.host_header.clone());
    headers.insert(
        HeaderName::from_static("user-agent"),
        config.user_agent.clone(),
    );
    headers.insert(
        HeaderName::from_static("x-opencode-session"),
        config.session_header.clone(),
    );
}

/// Forwarding headers for translated bodies (chat/completions): all client
/// headers except auth/host and hop-by-hop / body-describing headers that
/// reqwest recalculates for the translated JSON body, plus the identity
/// headers.
fn build_upstream_headers(
    auth_header: &'static str,
    config: &ProxyConfig,
    req: &Request,
) -> HeaderMap {
    let mut headers = HeaderMap::new();
    for header in req.headers() {
        let name_lower = header.field.as_str().to_ascii_lowercase();
        // Strip auth (replaced below), host (replaced below), and body /
        // transport headers that reqwest recomputes for the translated
        // body sent via `.json()`. Forwarding a stale `content-length` or
        // `content-type` would mismatch the new body and cause truncation
        // or 400s upstream. The identity headers are also stripped — the
        // proxy's own values are injected below.
        if matches!(
            name_lower.as_str(),
            "authorization"
                | "host"
                | "content-length"
                | "content-type"
                | "transfer-encoding"
                | "connection"
                | "user-agent"
                | "x-opencode-session"
        ) {
            continue;
        }
        let Ok(header_name) = HeaderName::from_bytes(name_lower.as_bytes()) else {
            continue;
        };
        if let Ok(value) = HeaderValue::from_bytes(header.value.as_bytes()) {
            headers.append(header_name, value);
        }
    }
    apply_identity_headers(&mut headers, auth_header, config);
    headers
}

/// Relay a reqwest response back through tiny_http (passthrough).
///
/// This reads the entire upstream body into memory before forwarding. That is
/// acceptable for the bounded `/v1/models` list response and upstream error
/// bodies. A 200 GPT-family passthrough streams instead via the caller.
fn relay_response(
    req: Request,
    upstream_resp: reqwest::blocking::Response,
    kind: ProxyKind,
) -> Result<()> {
    let status = upstream_resp.status();
    let mut response_headers = Vec::new();
    for (name, value) in upstream_resp.headers().iter() {
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "trailer" | "upgrade"
        ) {
            continue;
        }
        if let Ok(h) = Header::from_bytes(name.as_str().as_bytes(), value.as_bytes()) {
            response_headers.push(h);
        }
    }

    let content_length = upstream_resp.content_length().and_then(|len| {
        if len <= usize::MAX as u64 {
            Some(len as usize)
        } else {
            None
        }
    });

    let response = Response::new(
        StatusCode(status.as_u16()),
        response_headers,
        Box::new(upstream_resp) as Box<dyn Read + Send>,
        content_length,
        None,
    );
    if let Err(e) = req.respond(response) {
        eprintln!("{}: failed to relay response: {e}", kind.as_str());
    }
    Ok(())
}

#[cfg(test)]
mod route_tests {
    use super::RouteKind;
    use super::classify_route;
    use super::kind_for_base;
    use codex_proxy_protocol::ProxyKind;
    use pretty_assertions::assert_eq;

    #[test]
    fn models_route_ignores_query_string() {
        assert_eq!(
            classify_route("/v1/models?client_version=0.121.0"),
            RouteKind::Models
        );
        assert_eq!(classify_route("/v1/models"), RouteKind::Models);
        assert_eq!(classify_route("/models"), RouteKind::Models);
    }

    #[test]
    fn other_routes_classify() {
        assert_eq!(classify_route("/v1/responses"), RouteKind::Responses);
        assert_eq!(classify_route("/health"), RouteKind::Health);
        assert_eq!(classify_route("/shutdown"), RouteKind::Shutdown);
        assert_eq!(classify_route("/nope"), RouteKind::Forbidden);
        assert_eq!(classify_route("/v1/unknown?x=1"), RouteKind::Forbidden);
    }

    #[test]
    fn base_path_selects_kind() {
        assert_eq!(
            kind_for_base("https://opencode.ai/zen/v1"),
            ProxyKind::OpencodeZen
        );
        assert_eq!(
            kind_for_base("https://opencode.ai/zen/go/v1"),
            ProxyKind::OpencodeGo
        );
        assert_eq!(
            kind_for_base("http://127.0.0.1:8080"),
            ProxyKind::OpencodeZen
        );
    }
}
