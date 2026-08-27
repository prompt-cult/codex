//! `codex-mistral-proxy` — A standalone translating proxy for Mistral AI.
//!
//! Accepts OAI Responses API requests from codex-rs and routes them to the
//! Mistral Chat Completions API (`POST {upstream}/chat/completions`),
//! translating the upstream SSE stream back to OAI Responses SSE. Also exposes
//! `GET /v1/models` so the main app can discover the set of models Mistral
//! currently exposes (e.g. `zai-glm-5-2`, `mistral-large-latest`,
//! `mistral-medium-latest`, `devstral`, and any future additions).
//!
//! This crate mirrors the structure and security model of `codex-zen-proxy`:
//! the API key is read from stdin, stored in `mlock(2)`-protected memory,
//! and injected into upstream requests. The user boots the right proxy
//! themselves; the proxy is not auto-spawned by the app.
//!
//! Mistral-only: there is no model-family routing — every request goes to the
//! Mistral upstream. Any future model Mistral exposes works automatically.

use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
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
use globset::GlobSet;
use reqwest::Url;
use reqwest::blocking::Client;
use reqwest::header::AUTHORIZATION;
use reqwest::header::HOST;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use serde::Serialize;
use tiny_http::Header;
use tiny_http::Method;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;
use tiny_http::StatusCode;

mod models_translate;
mod read_api_key;
mod translate_request;
mod translate_sse;

use read_api_key::read_auth_header;

/// This proxy's protocol identity; prefixes every log line and names the
/// config file. See `docs/proxy-protocol.md`.
const KIND: ProxyKind = ProxyKind::MistralAi;

const DEFAULT_UPSTREAM_BASE: &str = "https://api.mistral.ai/v1";

/// Compiled-in defaults used when no `proxy-mistral-ai.jsonc` exists. The
/// exclusion list drops non-chat-purpose families from discovery; the bare
/// alias `glm-5-2` is exact-matched so `zai-glm-5-2` still passes.
const MISTRAL_DEFAULTS: ProxyDefaults = ProxyDefaults {
    upstream_base_url: DEFAULT_UPSTREAM_BASE,
    model_exclude_globs: &[
        "*-ocr-*",
        "*-mini-*",
        "magistral-*",
        "ministral-*",
        "voxtral-*",
        "glm-5-2",
    ],
};

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

/// CLI arguments for the Mistral translating proxy.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "codex-mistral-proxy",
    about = "Translating proxy: OAI Responses API ↔ Mistral Chat Completions"
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

    /// Base URL of the Mistral API. Overrides `upstream_base_url` in
    /// `proxy-mistral-ai.jsonc`; default: https://api.mistral.ai/v1.
    /// Chat requests go to `{base}/chat/completions`, model listings to `{base}/models`.
    #[arg(long)]
    pub upstream_base: Option<String>,
}

#[derive(Serialize)]
struct ServerInfo {
    port: u16,
    pid: u32,
}

struct ProxyConfig {
    /// Base URL (no trailing slash), e.g. "https://api.mistral.ai/v1"
    upstream_base: String,
    /// Pre-parsed host header value for upstream requests
    host_header: HeaderValue,
    /// Glob exclusions applied to discovered model IDs.
    exclude: GlobSet,
    /// Per-model metadata overrides from the proxy config file.
    model_overrides: std::collections::HashMap<String, models_translate::ModelTranslateOverride>,
    /// Logging verbosity from the proxy config file.
    log_level: LogLevel,
    /// Per-process request counter for verbose routing logs.
    request_counter: AtomicU64,
}

/// Entry point.
pub fn run_main(args: Args) -> Result<()> {
    let auth_header = read_auth_header()?;

    let config_dir = codex_utils_home_dir::find_codex_home()
        .context("resolving codex config dir for proxy config")?;
    let resolved = load_config(KIND, config_dir.as_path(), &MISTRAL_DEFAULTS)?;
    match &resolved.loaded_from {
        Some(path) => eprintln!("{}: loaded config from {}", KIND.as_str(), path.display()),
        None => eprintln!(
            "{}: no {} found, using built-in defaults",
            KIND.as_str(),
            KIND.config_filename()
        ),
    }

    // Precedence: CLI flag > config file > compiled-in default.
    let upstream_base = args
        .upstream_base
        .or(resolved.upstream_base_url.clone())
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
                models_translate::ModelTranslateOverride {
                    base_instructions: ovr.base_instructions.clone(),
                },
            )
        })
        .collect();

    let config = Arc::new(ProxyConfig {
        upstream_base,
        host_header,
        exclude: resolved.exclude,
        model_overrides,
        log_level: resolved.log_level,
        request_counter: AtomicU64::new(0),
    });

    let (listener, bound_addr) = bind_listener(args.port)?;
    if let Some(path) = args.server_info.as_ref() {
        write_server_info(path, bound_addr.port())?;
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
        KIND.as_str(),
        config.upstream_base
    );

    let http_shutdown = args.http_shutdown;
    for request in server.incoming_requests() {
        let client = client.clone();
        let config = config.clone();
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
                    "proxy": KIND.as_str(),
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

            if let Err(e) = handle_request(&client, auth_header, &config, request) {
                eprintln!("{} error: {e}", KIND.as_str());
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

fn write_server_info(path: &Path, port: u16) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let info = ServerInfo {
        port,
        pid: std::process::id(),
    };
    let mut data = serde_json::to_string(&info)?;
    data.push('\n');
    let mut f = File::create(path)?;
    f.write_all(data.as_bytes())?;
    Ok(())
}

fn handle_request(
    client: &Client,
    auth_header: &'static str,
    config: &ProxyConfig,
    req: Request,
) -> Result<()> {
    let method = req.method().clone();
    let url = req.url().to_string();
    let route = classify_route(&url);

    eprintln!("{}: {method} {url} -> {route:?}", KIND.as_str());

    // GET /v1/models — translate Mistral's model list into the codex
    // `ModelsResponse` shape for dynamic discovery by the main app.
    if method == Method::Get && route == RouteKind::Models {
        return handle_models_request(client, auth_header, config, req);
    }

    // POST /v1/responses — translate to Mistral chat/completions.
    if method == Method::Post && route == RouteKind::Responses {
        return handle_responses_translate(client, auth_header, config, req);
    }

    eprintln!("{}: 403 forbidden for {method} {url}", KIND.as_str());
    if let Err(e) = req.respond(Response::new_empty(StatusCode(403))) {
        eprintln!("{}: failed to respond 403: {e}", KIND.as_str());
    }
    Ok(())
}

/// GET /v1/models: fetch `{upstream}/models` and translate Mistral's raw list
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
    eprintln!("{}: fetching upstream {upstream_url}", KIND.as_str());

    let mut headers = HeaderMap::new();
    let mut auth_value = HeaderValue::from_static(auth_header);
    auth_value.set_sensitive(true);
    headers.insert(AUTHORIZATION, auth_value);
    headers.insert(HOST, config.host_header.clone());

    let upstream_resp = client
        .get(&upstream_url)
        .headers(headers)
        .send()
        .context("forwarding models request to upstream")?;

    eprintln!(
        "{}: upstream responded {}",
        KIND.as_str(),
        upstream_resp.status()
    );

    // Relay non-200 responses verbatim so the app can log the real cause.
    if upstream_resp.status().as_u16() != 200 {
        return relay_response(req, upstream_resp);
    }

    let raw = upstream_resp
        .bytes()
        .context("reading Mistral models response")?;
    let translated =
        models_translate::translate_mistral_models(&raw, &config.exclude, &config.model_overrides)
            .context("translating Mistral /models to ModelsResponse")?;
    let kept = translated.response.models.len();
    let data = serde_json::to_vec(&translated.response).context("serializing ModelsResponse")?;

    eprintln!(
        "{}: loaded {} models, {} after exclusions",
        KIND.as_str(),
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
        eprintln!("{}: failed to respond models: {e}", KIND.as_str());
    }
    Ok(())
}

/// Translate OAI Responses API request to Mistral Chat Completions and back.
fn handle_responses_translate(
    client: &Client,
    auth_header: &'static str,
    config: &ProxyConfig,
    mut req: Request,
) -> Result<()> {
    let fwd_headers = build_upstream_headers(auth_header, config, &req);

    let mut body_bytes = Vec::new();
    req.as_reader().read_to_end(&mut body_bytes)?;

    let body: serde_json::Value =
        serde_json::from_slice(&body_bytes).context("parsing request JSON")?;
    let model = body["model"].as_str().unwrap_or("").to_string();
    let is_stream = body["stream"].as_bool().unwrap_or(false);

    let verbose = config.log_level == LogLevel::Verbose;
    let req_id = config.request_counter.fetch_add(1, Ordering::Relaxed) + 1;
    if verbose {
        eprintln!(
            "{}: req#{req_id} model={model} stream={is_stream}",
            KIND.as_str()
        );
    }

    let mistral_body = translate_request::oai_to_mistral(&body);
    let upstream_url = format!("{}/chat/completions", config.upstream_base);

    let upstream_resp = client
        .post(&upstream_url)
        .headers(fwd_headers)
        .json(&mistral_body)
        .send()
        .context("forwarding Mistral chat request to upstream")?;

    if upstream_resp.status().as_u16() != 200 {
        return relay_response(req, upstream_resp);
    }

    if !is_stream {
        let mistral_body: serde_json::Value =
            upstream_resp.json().context("reading Mistral response")?;
        if verbose {
            let upstream_model = mistral_body["model"].as_str().unwrap_or("");
            eprintln!(
                "{}: req#{req_id} upstream_model={upstream_model}",
                KIND.as_str()
            );
        }
        let oai_resp = translate_request::mistral_response_to_oai(&mistral_body, &model);
        let data = serde_json::to_vec(&oai_resp).unwrap_or_default();
        let resp = Response::from_data(data)
            .with_status_code(StatusCode(200))
            .with_header(
                Header::from_bytes(b"content-type", b"application/json")
                    .unwrap_or_else(|_| unreachable!()),
            );
        if let Err(e) = req.respond(resp) {
            eprintln!("{}: failed to respond models: {e}", KIND.as_str());
        }
        return Ok(());
    }

    // Streaming: translate Mistral Chat SSE → OAI Responses SSE.
    let verbose_req = verbose.then_some(req_id);
    let translator = translate_sse::MistralToOaiStream::new(model, upstream_resp, verbose_req);
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
        eprintln!("{}: failed to respond stream: {e}", KIND.as_str());
    }
    Ok(())
}

/// Extract forwarding headers from an incoming request (all except auth/host
/// and hop-by-hop / body-describing headers that reqwest recalculates for the
/// translated JSON body).
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
        // `mistral_body` sent via `.json()`. Forwarding a stale
        // `content-length` or `content-type` would mismatch the new body and
        // cause truncation or 400s upstream.
        if matches!(
            name_lower.as_str(),
            "authorization"
                | "host"
                | "content-length"
                | "content-type"
                | "transfer-encoding"
                | "connection"
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
    let mut auth_value = HeaderValue::from_static(auth_header);
    auth_value.set_sensitive(true);
    headers.insert(AUTHORIZATION, auth_value);
    headers.insert(HOST, config.host_header.clone());
    headers
}

/// Relay a reqwest response back through tiny_http (passthrough).
///
/// This reads the entire upstream body into memory before forwarding. That is
/// acceptable for the bounded `/v1/models` list response, which is the only
/// current caller. If a future passthrough endpoint returns a chunked or
/// unbounded streaming body, switch to a streaming relay to avoid blocking
/// until the upstream stream closes.
fn relay_response(req: Request, upstream_resp: reqwest::blocking::Response) -> Result<()> {
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
        eprintln!("{}: failed to relay response: {e}", KIND.as_str());
    }
    Ok(())
}

#[cfg(test)]
mod route_tests {
    use super::RouteKind;
    use super::classify_route;
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
}
