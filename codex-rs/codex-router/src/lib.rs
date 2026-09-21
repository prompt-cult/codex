//! `codex-proxy-router` — routing dispatcher for the secure translating proxies.
//!
//! Listens on `127.0.0.1` and routes each request by its first path segment —
//! the upstream host name — to the matching translating proxy, which it boots
//! as a child process:
//!
//! - `POST /opencode.ai/v1/responses` → `codex-zen-proxy` child as `POST /v1/responses`
//! - `GET  /mistral.ai/v1/models?client_version=X` → `codex-mistral-proxy` child
//!
//! The remainder of the path (including any query string) is passed through
//! untouched, request headers and bodies are relayed verbatim, and response
//! bytes — including SSE streams — are relayed straight back without buffering
//! or parsing.
//!
//! The dispatcher holds no key material and never sees any. Each child proxy
//! owns its own hardened key state (stdin- or environment-vaulted today;
//! workload identities — KMS / k8s volume mounts / hardened key stores —
//! later). The dispatcher's only secret affordance is an allow-list of
//! environment variable names copied opaquely from its own environment into
//! the child's sanitized environment at spawn; see `README.proxy-router.md`.

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use clap::Parser;
use reqwest::blocking::Client;
use reqwest::header::HeaderMap;
use reqwest::header::HeaderName;
use reqwest::header::HeaderValue;
use serde::Serialize;
use std::fs;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::net::TcpListener;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;
use tiny_http::Header;
use tiny_http::Method;
use tiny_http::Request;
use tiny_http::Response;
use tiny_http::Server;
use tiny_http::StatusCode;

mod backend;

use backend::BACKENDS;
use backend::Backend;

/// How long a child proxy may take to publish its server-info before the
/// dispatcher gives up at startup.
const CHILD_BOOT_TIMEOUT: Duration = Duration::from_secs(30);

/// CLI arguments for the routing dispatcher.
#[derive(Debug, Clone, Parser)]
#[command(
    name = "codex-proxy-router",
    about = "Routing dispatcher: routes /<upstream-host>/... to hardened translating proxies"
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
}

#[derive(Serialize)]
struct ServerInfo {
    port: u16,
    pid: u32,
}

/// A running child proxy; the dispatcher terminates it when this is dropped.
/// The `Child` sits in a `Mutex` so the /shutdown path can kill and reap it
/// through the shared `Arc<Router>`.
struct ChildProxy {
    label: &'static str,
    port: u16,
    child: Mutex<Child>,
    tmp_dir: PathBuf,
}

impl Drop for ChildProxy {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = fs::remove_dir_all(&self.tmp_dir);
    }
}

/// All running children plus the shared blocking HTTP client.
struct Router {
    children: Vec<ChildProxy>,
    client: Arc<Client>,
}

impl Router {
    /// Kills and reaps every child proxy. Called on the /shutdown path
    /// before process exit: `std::process::exit` skips destructors, so
    /// termination must be explicit.
    fn terminate_children(&self) {
        for child in &self.children {
            if let Ok(mut child) = child.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = fs::remove_dir_all(&child.tmp_dir);
        }
    }
}

/// Classification of an incoming request path.
///
/// The remainder is the path remainder plus any query string; a bare prefix
/// (with or without a query) forwards to "/".
#[derive(Debug, PartialEq, Eq)]
enum Route {
    Backend(&'static Backend, String),
    Health,
    Shutdown,
    Unknown,
}

/// Map a raw request URL (path plus optional query) to a [`Route`].
fn classify(raw_url: &str) -> Route {
    let path = raw_url.split('?').next().unwrap_or(raw_url);
    if path == "/health" {
        return Route::Health;
    }
    if path == "/shutdown" {
        return Route::Shutdown;
    }
    for backend in BACKENDS {
        let Some(rest) = path.strip_prefix(backend.prefix) else {
            continue;
        };
        if rest.is_empty() || rest.starts_with('/') {
            let mut remainder = if rest.is_empty() {
                "/".to_string()
            } else {
                rest.to_string()
            };
            if let Some((_, query)) = raw_url.split_once('?') {
                remainder.push('?');
                remainder.push_str(query);
            }
            return Route::Backend(backend, remainder);
        }
    }
    Route::Unknown
}

/// Entry point.
pub fn run_main(args: Args) -> Result<()> {
    let exe_dir = std::env::current_exe()
        .context("resolving own path")?
        .parent()
        .map(Path::to_path_buf)
        .context("resolving own directory")?;

    let children = BACKENDS
        .iter()
        .map(|backend| boot_child(backend, &exe_dir))
        .collect::<Result<Vec<_>>>()?;

    let (listener, listener_addr) = bind_listener(args.port)?;
    if let Some(path) = args.server_info.as_ref() {
        write_server_info(path, listener_addr.port())?;
    }
    let server = Server::from_listener(listener, None)
        .map_err(|err| anyhow!("creating HTTP server: {err}"))?;
    let client = Arc::new(
        Client::builder()
            .timeout(None::<Duration>)
            .build()
            .context("building blocking HTTP client")?,
    );
    let router = Arc::new(Router { children, client });

    eprintln!("routing on {listener_addr}");
    for backend in BACKENDS {
        eprintln!(
            "  {}/... -> {} (child {})",
            backend.prefix, backend.label, backend.binary
        );
    }

    let http_shutdown = args.http_shutdown;
    for request in server.incoming_requests() {
        let router = router.clone();
        std::thread::spawn(move || {
            let method = request.method().clone();
            let raw_url = request.url().to_string();

            if http_shutdown && method == Method::Get && classify(&raw_url) == Route::Shutdown {
                let _ = request.respond(Response::new_empty(StatusCode(200)));
                router.terminate_children();
                std::process::exit(0);
            }

            if method == Method::Get && classify(&raw_url) == Route::Health {
                let body = serde_json::json!({
                    "status": "ok",
                    "proxy": "codex-proxy-router",
                    "backends": BACKENDS.iter().map(|backend| serde_json::json!({
                        "prefix": backend.prefix,
                        "binary": backend.binary,
                    })).collect::<Vec<_>>(),
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

            if let Err(e) = handle_request(&router, &method, &raw_url, request) {
                eprintln!("router: {e}");
            }
        });
    }

    Err(anyhow!("dispatcher stopped unexpectedly"))
}

/// Reaps a child whose boot did not complete and removes its temp dir.
fn reap_failed_boot(child: &mut Child, tmp_dir: &std::path::Path) {
    let _ = child.kill();
    let _ = child.wait();
    let _ = fs::remove_dir_all(tmp_dir);
}

/// Boots one backend child proxy from its sibling executable.
///
/// The child's environment is sanitized to the backend's allow-list: only
/// names listed in `allowed_env` that exist (non-empty) in the dispatcher's
/// own environment are inherited, opaquely. stdin is closed so a child that
/// cannot resolve its key from its vault fails fast instead of blocking. If
/// the boot does not complete, the child is reaped and its temp dir removed.
fn boot_child(backend: &'static Backend, exe_dir: &Path) -> Result<ChildProxy> {
    let exe_path = exe_dir.join(backend.binary);
    anyhow::ensure!(
        exe_path.is_file(),
        "{}: executable not found next to the dispatcher binary at {}",
        backend.binary,
        exe_path.display()
    );

    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp_dir = std::env::temp_dir().join(format!(
        "proxy-proxy-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::SeqCst)
    ));
    fs::create_dir_all(&tmp_dir)
        .with_context(|| format!("creating temp dir {}", tmp_dir.display()))?;
    let info_path = tmp_dir.join("server.json");

    let mut cmd = Command::new(&exe_path);
    cmd.args(["--http-shutdown", "--server-info"])
        .arg(&info_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .env_clear();
    for name in backend.allowed_env {
        if let Ok(value) = std::env::var(name)
            && !value.is_empty()
        {
            cmd.env(name, value);
        }
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            let _ = fs::remove_dir_all(&tmp_dir);
            anyhow::bail!("booting {}: {err}", exe_path.display());
        }
    };

    let deadline = Instant::now() + CHILD_BOOT_TIMEOUT;
    let port = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                reap_failed_boot(&mut child, &tmp_dir);
                anyhow::bail!(
                    "{} exited early with {status} before publishing server-info; it could not obtain its API key",
                    backend.binary
                );
            }
            Ok(None) => {}
            Err(err) => {
                reap_failed_boot(&mut child, &tmp_dir);
                anyhow::bail!("polling {}: {err}", backend.binary);
            }
        }
        if let Ok(text) = fs::read_to_string(&info_path) {
            let port = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .and_then(|v| v["port"].as_u64())
                .map(|p| p as u16)
                .unwrap_or(0);
            if port != 0 {
                break port;
            }
        }
        if Instant::now() >= deadline {
            reap_failed_boot(&mut child, &tmp_dir);
            anyhow::bail!(
                "{} did not publish server-info within {CHILD_BOOT_TIMEOUT:?}",
                backend.binary
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    };

    Ok(ChildProxy {
        label: backend.label,
        port,
        child: Mutex::new(child),
        tmp_dir,
    })
}

fn bind_listener(port: Option<u16>) -> Result<(TcpListener, std::net::SocketAddr)> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
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

fn handle_request(router: &Router, method: &Method, raw_url: &str, req: Request) -> Result<()> {
    match *method {
        Method::Post | Method::Get => {}
        Method::Head
        | Method::Put
        | Method::Delete
        | Method::Options
        | Method::Connect
        | Method::Patch
        | Method::Trace
        | Method::NonStandard(_) => {
            return respond_error(req, 405, "router_method_not_allowed", "method not allowed");
        }
    }

    let route = classify(raw_url);
    match route {
        Route::Backend(backend, remainder) => {
            let Some(port) = router
                .children
                .iter()
                .find(|proxy| proxy.label == backend.label)
                .map(|proxy| proxy.port)
            else {
                return respond_404(req);
            };
            forward(router, method, port, &remainder, req)
        }
        Route::Health | Route::Shutdown | Route::Unknown => respond_404(req),
    }
}

/// Relay an incoming request to the backend child at `port`, then relay the
/// response bytes straight back (streaming; SSE preserved).
fn forward(
    router: &Router,
    method: &Method,
    port: u16,
    remainder: &str,
    mut req: Request,
) -> Result<()> {
    let url = format!("http://127.0.0.1:{port}{remainder}");
    let fwd_headers = build_forward_headers(&req);

    let mut body_bytes = Vec::new();
    req.as_reader().read_to_end(&mut body_bytes)?;

    let builder = match *method {
        Method::Post => router.client.post(&url),
        Method::Get => router.client.get(&url),
        _ => unreachable!("handle_request gates methods to Post/Get"),
    };

    let upstream = match builder.headers(fwd_headers).body(body_bytes).send() {
        Ok(upstream) => upstream,
        Err(err) => {
            return respond_error(
                req,
                502,
                "router_bad_gateway",
                &format!("backend child unreachable: {err}"),
            );
        }
    };
    relay_response(req, upstream)
}

/// Headers that are hop-by-hop by definition plus the transport ones
/// reqwest recalculates; none of them may reach a backend child.
const HOP_BY_HOP: &[&str] = &[
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "te",
    "trailer",
    "upgrade",
    "keep-alive",
    "proxy-connection",
    "proxy-authenticate",
    "proxy-authorization",
];

/// Copies every end-to-end request header to the backend child: skips
/// hop-by-hop headers and any header named in `Connection`. No key material
/// is injected here: the backend child injects its own vaulted API key.
fn build_forward_headers(req: &Request) -> HeaderMap {
    let mut conn_named: Vec<String> = Vec::new();
    for header in req.headers() {
        if header.field.as_str().to_ascii_lowercase() == "connection"
            && let Ok(value) = std::str::from_utf8(header.value.as_bytes())
        {
            conn_named.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .map(str::to_ascii_lowercase)
                    .filter(|name| !name.is_empty()),
            );
        }
    }

    let mut headers = HeaderMap::new();
    for header in req.headers() {
        let name_lower = header.field.as_str().to_ascii_lowercase();
        if HOP_BY_HOP.contains(&name_lower.as_str())
            || conn_named.iter().any(|name| name == &name_lower)
        {
            continue;
        }
        let Ok(header_name) = HeaderName::from_bytes(name_lower.as_bytes()) else {
            continue;
        };
        if let Ok(value) = HeaderValue::from_bytes(header.value.as_bytes()) {
            headers.append(header_name, value);
        }
    }
    headers
}

fn respond_error(req: Request, status: u16, code: &str, message: &str) -> Result<()> {
    let err_body = serde_json::json!({
        "error": {
            "message": message,
            "type": code,
        }
    });
    let data = serde_json::to_vec(&err_body).unwrap_or_default();
    let resp = Response::from_data(data)
        .with_status_code(StatusCode(status))
        .with_header(
            Header::from_bytes(b"content-type", b"application/json")
                .unwrap_or_else(|_| unreachable!()),
        );
    let _ = req.respond(resp);
    Ok(())
}

fn respond_404(req: Request) -> Result<()> {
    respond_error(
        req,
        404,
        "router_unknown_backend",
        "no backend for this path",
    )
}

/// Relay a blocking response back through tiny_http (streaming passthrough).
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

    let data_length = upstream_resp.content_length().and_then(|len| {
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
        data_length,
        None,
    );
    let _ = req.respond(response);
    Ok(())
}

#[cfg(test)]
mod classify_tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn backend_routes_strip_prefix_and_keep_query() {
        assert_eq!(
            classify("/opencode.ai/v1/responses"),
            Route::Backend(&BACKENDS[0], "/v1/responses".to_string())
        );
        assert_eq!(
            classify("/mistral.ai/v1/models?client_version=0.121.0"),
            Route::Backend(
                &BACKENDS[1],
                "/v1/models?client_version=0.121.0".to_string()
            )
        );
    }

    #[test]
    fn bare_prefix_routes_to_root() {
        assert_eq!(
            classify("/opencode.ai"),
            Route::Backend(&BACKENDS[0], "/".to_string())
        );
        assert_eq!(
            classify("/opencode.ai/"),
            Route::Backend(&BACKENDS[0], "/".to_string())
        );
    }

    #[test]
    fn bare_prefix_with_query_forwards_root_path() {
        assert_eq!(
            classify("/opencode.ai?x=1"),
            Route::Backend(&BACKENDS[0], "/?x=1".to_string())
        );
    }

    #[test]
    fn similar_prefixes_do_not_match() {
        assert_eq!(classify("/opencode.ai.evil.com/v1"), Route::Unknown);
    }

    #[test]
    fn meta_routes_classify() {
        assert_eq!(classify("/health"), Route::Health);
        assert_eq!(classify("/shutdown"), Route::Shutdown);
        assert_eq!(classify("/nope?x=1"), Route::Unknown);
    }
}
