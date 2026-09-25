//! End-to-end breakage oracle for the opencode proxy.
//!
//! Boots the real `codex-opencode-proxy` binary against a local mock upstream
//! and drives every externally visible behaviour: health, GPT passthrough
//! (byte-for-byte), Claude Anthropic translation, Chat Completions translation
//! (stream + non-stream), model discovery with config-file exclusions, the
//! Go-endpoint identity headers (User-Agent + x-opencode-session), auth
//! injection, route guarding, and clean shutdown.

use std::io::prelude::*;
use std::net::TcpListener;
use std::net::TcpStream;
use std::process;
use std::process::Child;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const KEY: &str = "opencode-test-key-456";
const AUTH: &str = "Bearer opencode-test-key-456";

/// Bytes the mock returns for a GPT-family request at {base}/responses.
/// The proxy must pass these through verbatim.
const GPT_PASSTHROUGH_SSE: &str = concat!(
    "event: response.output_text.delta\n",
    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n",
    "\n",
    "event: response.completed\n",
    "data: {\"type\":\"response.completed\"}\n",
    "\n",
);

/// Anthropic Messages SSE served at {base}/messages for Claude-family models;
/// the proxy must translate this into OAI Responses SSE.
const CLAUDE_SSE: &str = concat!(
    "data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"role\":\"assistant\"}}\n",
    "\n",
    "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n",
    "\n",
    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n",
    "\n",
    "data: {\"type\":\"content_block_stop\",\"index\":0}\n",
    "\n",
    "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}\n",
    "\n",
    "data: {\"type\":\"message_stop\"}\n",
    "\n",
);

/// Chat Completions SSE served at {base}/chat/completions for Chat-family
/// models when the translated request has `stream:true`.
const CHAT_SSE: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"1 2 3\"}}]}\n",
    "\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],",
    "\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n",
    "\n",
    "data: [DONE]\n",
    "\n",
);

/// Reasoning-model streaming fixture (captured live from zai-glm-5-3):
/// thinking block deltas first, then a final mixed block list with the text.
const CHAT_SSE_BLOCKS: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":[",
    "{\"type\":\"thinking\",\"thinking\":[{\"type\":\"text\",\"text\":\"The user is asking\"}],\"closed\":true}]}}]}\n",
    "\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":[",
    "{\"type\":\"thinking\",\"thinking\":[{\"type\":\"text\",\"text\":\" what to reply\"}],\"closed\":true}]}}]}\n",
    "\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":[",
    "{\"type\":\"thinking\",\"thinking\":[{\"type\":\"text\",\"text\":\"…\"}],\"closed\":true},",
    "{\"type\":\"text\",\"text\":\"pong zai-glm-5-3\"}]}}]}\n",
    "\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],",
    "\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n",
    "\n",
    "data: [DONE]\n",
    "\n",
);

/// Chat Completions JSON for non-streaming Chat-family requests.
const CHAT_JSON: &str = r#"{
  "id": "chatcmpl-1",
  "model": "glm-5.3-flash",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "PROXY OK"}, "finish_reason": "stop"}],
  "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
}"#;

/// Upstream /models payload; `chat-embed` is not chat-capable and must be
/// filtered out by discovery translation.
const MODELS_FIXTURE: &str = r#"{
  "object": "list",
  "data": [
    {"id": "zai-glm-5-2", "object": "model",
     "capabilities": {"completion_chat": true, "function_calling": true},
     "max_context_length": 131072},
    {"id": "chat-medium-latest", "object": "model", "capabilities": {"completion_chat": true}},
    {"id": "chat-embed", "object": "model", "capabilities": {"completion_chat": false}}
  ]
}"#;

#[derive(Debug, Clone)]
struct RecordedRequest {
    method: String,
    url: String,
    authorization: Option<String>,
    user_agent: Option<String>,
    session: Option<String>,
    body: String,
}

fn recordings() -> Arc<Mutex<Vec<RecordedRequest>>> {
    Arc::new(Mutex::new(Vec::new()))
}

/// A mock OpenCode upstream. Every request must carry the expected
/// Authorization header (or x-api-key for the Anthropic route) or it gets a
/// 401 — this proves the proxy injects the piped key. Requests are recorded so
/// tests can assert on the identity headers the proxy must add.
struct MockUpstream {
    port: u16,
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

fn start_mock() -> MockUpstream {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock bind");
    let _port = listener.local_addr().unwrap().port();
    let requests = recordings();
    let thread_requests = requests.clone();
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut request in tiny_http::Server::from_listener(listener, None)
            .unwrap()
            .incoming_requests()
        {
            let method = request.method().clone();
            let url = request.url().to_string();
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);

            let header_value = |name: &str| -> Option<String> {
                request
                    .headers()
                    .iter()
                    .find(|h| h.field.as_str() == name)
                    .map(|h| h.value.as_str().to_string())
            };
            let auth = header_value("authorization");
            let api_key = header_value("x-api-key");
            let user_agent = header_value("user-agent");
            let session = header_value("x-opencode-session");

            let authed = auth.as_deref() == Some(AUTH) || api_key.as_deref() == Some(KEY);
            if !authed {
                let _ = request.respond(
                    tiny_http::Response::from_string("unauthorized").with_status_code(401),
                );
                continue;
            }

            thread_requests.lock().unwrap().push(RecordedRequest {
                method: method.to_string(),
                url: url.clone(),
                authorization: auth,
                user_agent,
                session,
                body: body.clone(),
            });

            let (status, content_type, payload) = if method == tiny_http::Method::Get
                && url.starts_with("/models")
            {
                (200, "application/json", MODELS_FIXTURE.to_string())
            } else if method == tiny_http::Method::Post && url.starts_with("/chat/completions") {
                if body.contains("kimi-k2") {
                    (200, "text/event-stream", CHAT_SSE_BLOCKS.to_string())
                } else if body.contains("\"stream\":true") {
                    (200, "text/event-stream", CHAT_SSE.to_string())
                } else {
                    (200, "application/json", CHAT_JSON.to_string())
                }
            } else if method == tiny_http::Method::Post && url.starts_with("/responses") {
                (200, "text/event-stream", GPT_PASSTHROUGH_SSE.to_string())
            } else if method == tiny_http::Method::Post && url.starts_with("/messages") {
                (200, "text/event-stream", CLAUDE_SSE.to_string())
            } else {
                (404, "text/plain", "unknown upstream route".to_string())
            };

            let mut resp = tiny_http::Response::from_string(payload).with_status_code(status);
            if let Ok(ct) = tiny_http::Header::from_bytes(&b"content-type"[..], content_type) {
                resp.add_header(ct);
            }
            let _ = request.respond(resp);
        }
    });
    MockUpstream { port, requests }
}

struct Proxy {
    child: Option<Child>,
    port: u16,
    tmp: std::path::PathBuf,
    info_path: std::path::PathBuf,
}

/// Boot the proxy keyless in secret-push mode with an ephemeral boot token.
/// Returns the proxy plus the token so tests can drive the provisioning flow.
fn boot_proxy_secret_push(upstream_port: u16) -> (Proxy, String) {
    let tmp = std::env::temp_dir().join(format!(
        "opencode-e2e-sp-{}-{}",
        process::id(),
        upstream_port
    ));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let info = tmp.join("server.json");
    let token = "boot-token-0123456789abcdef";
    // The token travels via a file, never argv: `ps` exposes command lines.
    let token_path = tmp.join("boot-token");
    std::fs::write(&token_path, token).unwrap();

    let child = process::Command::new(env!("CARGO_BIN_EXE_codex-opencode-proxy"))
        .args([
            "--http-shutdown",
            "--server-info",
            info.to_str().unwrap(),
            "--upstream-base",
            &format!("http://127.0.0.1:{upstream_port}"),
            "--secret-channel",
            "secret-push",
            "--boot-token-file",
            token_path.to_str().unwrap(),
        ])
        .env("CODEX_CONFIG_DIR", &tmp)
        .env_remove("OPENCODE_API_KEY")
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .spawn()
        .expect("spawn proxy");

    let deadline = Instant::now() + Duration::from_secs(30);
    let port = loop {
        if let Ok(text) = std::fs::read_to_string(&info) {
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
            panic!("proxy did not publish server-info within 30s");
        }
        thread::sleep(Duration::from_millis(100));
    };

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match http(&format!("http://127.0.0.1:{port}/health"), "GET", None, "") {
            Ok((status, _)) if status == 200 => break,
            _ if Instant::now() >= deadline => panic!("proxy never became healthy"),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }
    let proxy = Proxy {
        child: Some(child),
        port,
        info_path: info,
        tmp,
    };
    (proxy, token.to_string())
}

/// Boot the real proxy binary with the key on stdin and an ephemeral port,
/// wait up to 30s for /health. `config_jsonc` is written to a fresh temp
/// CODEX_CONFIG_DIR as `proxy-opencode-zen.jsonc` so tests are hermetic w.r.t.
/// the developer's real `~/.codex` settings.
fn boot_proxy(upstream_port: u16, config_jsonc: Option<&str>) -> Proxy {
    let tmp =
        std::env::temp_dir().join(format!("opencode-e2e-{}-{}", process::id(), upstream_port));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    if let Some(config) = config_jsonc {
        std::fs::write(tmp.join("proxy-opencode-zen.jsonc"), config).unwrap();
    }
    let info = tmp.join("server.json");

    let mut child = process::Command::new(env!("CARGO_BIN_EXE_codex-opencode-proxy"))
        .args([
            "--http-shutdown",
            "--server-info",
            info.to_str().unwrap(),
            "--upstream-base",
            &format!("http://127.0.0.1:{upstream_port}"),
        ])
        .env("CODEX_CONFIG_DIR", &tmp)
        .env_remove("OPENCODE_API_KEY")
        .stdin(process::Stdio::piped())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .spawn()
        .expect("spawn proxy");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(KEY.as_bytes())
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(30);
    let port = loop {
        if let Ok(text) = std::fs::read_to_string(&info) {
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
            panic!("proxy did not publish server-info within 30s");
        }
        thread::sleep(Duration::from_millis(100));
    };

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match http(&format!("http://127.0.0.1:{port}/health"), "GET", None, "") {
            Ok((status, _)) if status == 200 => break,
            _ if Instant::now() >= deadline => panic!("proxy never became healthy"),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }
    Proxy {
        child: Some(child),
        port,
        info_path: info,
        tmp,
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

/// Minimal std-only HTTP client. Requests `Connection: close` and reads to
/// EOF so streaming bodies work without any extra dependency. `extra_headers`
/// is appended verbatim (each entry must already end with `\r\n`).
fn http(
    url: &str,
    method: &str,
    body: Option<&str>,
    extra_headers: &str,
) -> Result<(u16, String), String> {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match rest.split_once('/') {
        Some((h, p)) => (h.to_string(), format!("/{p}")),
        None => (rest.to_string(), "/".to_string()),
    };
    let mut stream = TcpStream::connect(&host_port).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n");
    req.push_str(extra_headers);
    match body {
        Some(b) => {
            req.push_str(&format!(
                "content-type: application/json\r\ncontent-length: {}\r\n\r\n",
                b.len()
            ));
            stream.write_all(req.as_bytes()).ok();
            stream.write_all(b.as_bytes()).ok();
        }
        None => {
            req.push_str("\r\n");
            stream.write_all(req.as_bytes()).ok();
        }
    }
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("read: {e}"))?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "no status line".to_string())?;
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

#[test]
fn server_info_advertises_protocol_v1_and_channels() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let text = std::fs::read_to_string(&proxy.info_path).expect("server-info file must exist");
    let info: serde_json::Value = serde_json::from_str(&text).expect("server-info must parse");
    assert_eq!(info["server_info_version"], 1);
    assert_eq!(info["protocol_version"], 1);
    assert_eq!(
        info["secret_channels"],
        serde_json::json!(["env-debug", "secret-push"])
    );
}

#[test]
fn secret_push_holds_requests_then_provisions() {
    let mock = start_mock();
    let (proxy, _token) = boot_proxy_secret_push(mock.port);

    // The boot-token file must be consumed and deleted by the proxy at
    // startup: the token exists only in memory from that point on.
    assert!(
        !proxy.tmp.join("boot-token").exists(),
        "boot token file must be removed by the proxy at startup"
    );

    // Pre-provisioning upstream forwarding returns 503 proxy_secret_pending.
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/models", proxy.port),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 503, "pre-provisioning must hold requests");
    assert!(body.contains("proxy_secret_pending"), "body: {body}");

    // Provisioning without the boot token is rejected.
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"secret-push","key_id":"primary","material":"opencode-test-key-456"}"#),
        "",
    )
    .unwrap();
    assert_eq!(status, 401, "missing boot token must be rejected");

    // Valid provisioning returns 204 and unblocks upstream forwarding.
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"secret-push","key_id":"primary","material":"opencode-test-key-456"}"#),
        "X-Router-Boot-Token: boot-token-0123456789abcdef\r\n",
    )
    .unwrap();
    assert_eq!(status, 204, "valid provisioning must return 204");

    // Replay is rejected.
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"secret-push","key_id":"primary","material":"another"}"#),
        "X-Router-Boot-Token: boot-token-0123456789abcdef\r\n",
    )
    .unwrap();
    assert_eq!(status, 409, "replay must be rejected");

    // After provisioning, upstream forwarding works with the pushed key.
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/models", proxy.port),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "post-provisioning must forward, body: {body}");
}

#[test]
fn secret_push_rejects_mismatched_channel_payload() {
    let mock = start_mock();
    let (proxy, _token) = boot_proxy_secret_push(mock.port);

    let (status, body) = http(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(
            r#"{"channel":"workload-identity","key_id":"primary","material":"opencode-test-key-456"}"#,
        ),
        "X-Router-Boot-Token: boot-token-0123456789abcdef\r\n",
    )
    .unwrap();
    assert_eq!(status, 400, "mismatched channel must be rejected");
    assert!(
        body.contains("proxy_secret_channel_mismatch"),
        "body: {body}"
    );

    // The rejected push must not have provisioned anything.
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/models", proxy.port),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 503, "rejected push must leave the proxy keyless");
    assert!(body.contains("proxy_secret_pending"), "body: {body}");
}

#[test]
fn secret_push_trims_padded_material() {
    let mock = start_mock();
    let (proxy, _token) = boot_proxy_secret_push(mock.port);

    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(
            r#"{"channel":"secret-push","key_id":"primary","material":"  opencode-test-key-456  "}"#,
        ),
        "X-Router-Boot-Token: boot-token-0123456789abcdef\r\n",
    )
    .unwrap();
    assert_eq!(status, 204, "padded material must be trimmed and accepted");

    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/v1/models", proxy.port),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "the trimmed key must authenticate upstream");
}

#[test]
fn workload_identity_channel_is_rejected_at_startup() {
    let output = process::Command::new(env!("CARGO_BIN_EXE_codex-opencode-proxy"))
        .args(["--secret-channel", "workload-identity"])
        .env_remove("OPENCODE_API_KEY")
        .stdin(process::Stdio::null())
        .output()
        .expect("spawn proxy");
    assert!(
        !output.status.success(),
        "an unimplemented channel must never be advertised or accepted"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("workload-identity"), "stderr: {stderr}");
}

#[test]
fn health_reports_upstream_and_kind() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/health", proxy.port),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 200);
    assert!(body.contains("\"status\":\"ok\""), "body: {body}");
    assert!(
        body.contains("\"proxy\":\"proxy-opencode-zen\""),
        "body: {body}"
    );
    assert!(
        body.contains(&format!("http://127.0.0.1:{}", mock.port)),
        "body: {body}"
    );
}

#[test]
fn gpt_passthrough_streams_verbatim() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let body = r#"{"model":"gpt-5.6-luna","stream":true,"input":[{"type":"message","role":"user","content":"hi"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(body),
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    // Passthrough: the mock's exact SSE bytes must emerge unchanged.
    assert!(
        body.contains("event: response.output_text.delta"),
        "body: {body}"
    );
    assert!(body.contains("\"delta\":\"hi\""), "body: {body}");
    assert!(body.contains("event: response.completed"), "body: {body}");
}

#[test]
fn claude_translate_streams_to_oai_sse() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let body = r#"{"model":"claude-sonnet-4","stream":true,"input":[{"type":"message","role":"user","content":"hi"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(body),
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("event: response.created"), "body: {body}");
    assert!(
        body.contains("event: response.output_text.delta"),
        "body: {body}"
    );
    assert!(
        body.contains("\"delta\":\"hello\""),
        "translated text missing: {body}"
    );
    assert!(body.contains("event: response.completed"), "body: {body}");
}

#[test]
fn nonstream_chat_translates_responses_api() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let req = r#"{"model":"glm-5.3-flash","stream":false,"input":[{"type":"message","role":"user","content":"say OK"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"status\":\"completed\""), "body: {body}");
    assert!(body.contains("PROXY OK"), "body: {body}");
    assert!(
        body.contains("\"input_tokens\":7"),
        "usage translation lost: {body}"
    );
    assert!(
        body.contains("\"output_tokens\":3"),
        "usage translation lost: {body}"
    );

    let recorded = mock.requests.lock().unwrap().clone();
    let chat_req = recorded
        .iter()
        .find(|r| r.url.starts_with("/chat/completions"))
        .expect("chat upstream request recorded");
    assert_eq!(chat_req.authorization.as_deref(), Some(AUTH));
    assert_eq!(
        chat_req.user_agent.as_deref(),
        Some(concat!("codex-opencode-proxy/", env!("CARGO_PKG_VERSION")))
    );
    let session = chat_req.session.as_deref().expect("session header sent");
    let uuid = uuid::Uuid::parse_str(session).expect("session is a UUID");
    assert_eq!(uuid.get_version_num(), 4, "session must be UUIDv4");
    // The translated body must have reached the mock as Chat Completions.
    assert!(chat_req.body.contains("\"model\":\"glm-5.3-flash\""));
    assert!(chat_req.body.contains("\"messages\""));
}

#[test]
fn stream_chat_emits_full_sse_sequence() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let req = r#"{"model":"glm-5.3-flash","stream":true,"input":[{"type":"message","role":"user","content":"count"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("event: response.created"), "body: {body}");
    assert!(body.contains("event: response.in_progress"), "body: {body}");
    assert!(
        body.contains("event: response.output_item.added"),
        "body: {body}"
    );
    assert!(
        body.contains("event: response.output_text.delta"),
        "body: {body}"
    );
    assert!(body.contains("\"delta\":\"1 2 3\""), "delta lost: {body}");
    assert!(
        body.contains("event: response.output_item.done"),
        "body: {body}"
    );
    assert!(body.contains("event: response.completed"), "body: {body}");
    assert!(!body.contains("\"status\":\"incomplete\""), "body: {body}");
    assert!(body.contains("\"status\":\"completed\""), "body: {body}");
}

#[test]
fn stream_reasoning_blocks_surface_text_through_proxied_sse() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    // kimi-k2 routes to the Chat family; the block-list SSE shape was
    // captured live from Mistral's zai-glm-5-3.
    let req = r#"{"model":"kimi-k2","stream":true,"input":[{"type":"message","role":"user","content":"ping"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(
        body.contains("event: response.reasoning_text.delta"),
        "reasoning deltas missing: {body}"
    );
    assert!(
        body.contains("\"delta\":\"The user is asking\""),
        "body: {body}"
    );
    assert!(
        body.contains("event: response.output_text.delta"),
        "body: {body}"
    );
    assert!(
        body.contains("\"delta\":\"pong zai-glm-5-3\""),
        "block text not surfaced: {body}"
    );
    assert!(
        body.contains("event: response.output_item.done"),
        "body: {body}"
    );
    assert!(
        body.contains("\"type\":\"reasoning\""),
        "reasoning item missing: {body}"
    );
    assert!(body.contains("event: response.completed"), "body: {body}");
    assert!(body.contains("\"status\":\"completed\""), "body: {body}");
}

#[test]
fn proxy_identity_headers_override_client_versions() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let req = r#"{"model":"glm-5.3-flash","stream":false,"input":[{"type":"message","role":"user","content":"hi"}]}"#;
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
        "user-agent: hostile-generic-sdk/9\r\nx-opencode-session: client-forged-session\r\n",
    )
    .unwrap();
    assert_eq!(status, 200);

    let recorded = mock.requests.lock().unwrap().clone();
    let chat_req = recorded
        .iter()
        .find(|r| r.url.starts_with("/chat/completions"))
        .expect("chat upstream request recorded");
    assert_eq!(
        chat_req.user_agent.as_deref(),
        Some(concat!("codex-opencode-proxy/", env!("CARGO_PKG_VERSION"))),
        "client UA must be overridden: {chat_req:?}"
    );
    let session = chat_req.session.as_deref().expect("session header sent");
    uuid::Uuid::parse_str(session).expect("proxy session must be a UUID, not the forged value");
}

#[test]
fn session_header_is_stable_across_requests() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let req = r#"{"model":"glm-5.3-flash","stream":false,"input":[{"type":"message","role":"user","content":"a"}]}"#;
    let (status, _) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
        "",
    )
    .unwrap();
    assert_eq!(status, 200);
    let (status, _) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
        "",
    )
    .unwrap();
    assert_eq!(status, 200);

    let recorded = mock.requests.lock().unwrap().clone();
    let sessions: Vec<Option<String>> = recorded
        .iter()
        .filter(|r| r.url.starts_with("/chat/completions"))
        .map(|r| r.session.clone())
        .collect();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0], sessions[1], "session id must be stable");
}

#[test]
fn models_with_query_and_config_exclusions_translate_to_codex_shape() {
    let mock = start_mock();
    let config = r#"{
        // jsonc comments must parse
        "model_exclude_globs": ["*-medium-*"],
    }"#;
    let proxy = boot_proxy(mock.port, Some(config));
    // The query is not optional: codex's ModelsClient always appends it, and
    // exact-match routing that ignores the query historically 403'd here.
    let (status, body) = http(
        &format!(
            "http://127.0.0.1:{}/v1/models?client_version=0.121.0",
            proxy.port
        ),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(
        body.contains("\"models\":[{"),
        "missing ModelsResponse array: {body}"
    );
    assert!(body.contains("\"slug\":\"zai-glm-5-2\""), "body: {body}");
    assert!(body.contains("\"context_window\":131072"), "body: {body}");
    // Non-chat models must be filtered out of the codex list.
    assert!(!body.contains("chat-embed"), "embed model leaked: {body}");
    // The config-file exclusion glob must have dropped chat-medium-latest.
    assert!(
        !body.contains("chat-medium-latest"),
        "excluded model leaked: {body}"
    );

    let recorded = mock.requests.lock().unwrap().clone();
    let models_req = recorded
        .iter()
        .find(|r| r.url.starts_with("/models"))
        .expect("models upstream request recorded");
    assert_eq!(models_req.authorization.as_deref(), Some(AUTH));
    assert!(models_req.user_agent.is_some());
    assert!(models_req.session.is_some());
}

#[test]
fn unknown_route_is_403() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port, None);
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/nope", proxy.port),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 403);
}

#[test]
fn shutdown_endpoint_terminates_process() {
    let mock = start_mock();
    let mut proxy = boot_proxy(mock.port, None);
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/shutdown", proxy.port),
        "GET",
        None,
        "",
    )
    .unwrap();
    assert_eq!(status, 200);
    let mut child = proxy.child.take().expect("child");
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait().expect("try_wait") {
            Some(code) => {
                assert!(code.success(), "exit status: {code:?}");
                break;
            }
            None => {
                if Instant::now() >= deadline {
                    panic!("proxy did not exit after /shutdown within 30s");
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    drop(proxy);
}
