//! End-to-end breakage oracle for the mistral proxy.
//!
//! Boots the real `codex-mistral-proxy` binary against a local mock Mistral
//! upstream and drives every externally visible behaviour: health, model
//! discovery (including the `?client_version=` query codex always sends),
//! request/response translation, streaming SSE translation, tool-call
//! roundtrips, auth injection, route guarding, and clean shutdown.
//!
//! If any of these fail after a dependency purge, the purge broke the proxy —
//! restore the code, do not paper over it.

use std::io::prelude::*;
use std::net::TcpListener;
use std::net::TcpStream;
use std::process;
use std::process::Child;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const KEY: &str = "test-key-abc123";
const AUTH: &str = "Bearer test-key-abc123";

const MODELS_FIXTURE: &str = r#"{
  "object": "list",
  "data": [
    {"id": "zai-glm-5-2", "object": "model",
     "capabilities": {"completion_chat": true, "function_calling": true},
     "max_context_length": 131072},
    {"id": "mistral-medium-latest", "object": "model", "capabilities": {"completion_chat": true}},
    {"id": "mistral-embed", "object": "model", "capabilities": {"completion_chat": false}}
  ]
}"#;

const CHAT_SSE_TEXT: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"1 2 3\"}}]}\n",
    "\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],",
    "\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n",
    "\n",
    "data: [DONE]\n",
    "\n",
);

const CHAT_JSON_TEXT: &str = r#"{
  "id": "chatcmpl-1",
  "model": "mistral-medium-latest",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "PROXY OK"}, "finish_reason": "stop"}],
  "usage": {"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10}
}"#;

const CHAT_JSON_TOOLS: &str = r#"{
  "id": "chatcmpl-2",
  "model": "mistral-medium-latest",
  "choices": [{"index": 0, "message": {"role": "assistant", "content": "", "tool_calls": [
    {"id": "call_xyz", "type": "function",
     "function": {"name": "get_weather", "arguments": "{\"city\": \"Paris\"}"}}
  ]}, "finish_reason": "tool_calls"}],
  "usage": {"prompt_tokens": 9, "completion_tokens": 4, "total_tokens": 13}
}"#;

const CHAT_SSE_TOOLS: &str = concat!(
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"tool_calls\":[",
    "{\"index\":0,\"id\":\"call_xyz\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"ci\"}}]}}]}\n",
    "\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[",
    "{\"index\":0,\"function\":{\"arguments\":\"ty\\\": \\\"Paris\\\"}\"}}]}}]}\n",
    "\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],",
    "\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4}}\n",
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

struct MockUpstream {
    #[allow(dead_code)]
    port: u16,
}

fn free_port() -> u16 {
    let l = TcpListener::bind(("127.0.0.1", 0)).expect("bind ephemeral");
    let p = l.local_addr().unwrap().port();
    drop(l);
    p
}

/// Start a mock Mistral upstream. Every request must carry the expected
/// Authorization header or it gets a 401 — this proves the proxy injects the
/// piped key. GET /models and POST /chat/completions are served from fixtures.
fn start_mock() -> MockUpstream {
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

            let authed = request
                .headers()
                .iter()
                .any(|h| h.field.equiv("Authorization") && h.value.as_str() == AUTH);
            if !authed {
                let _ = request.respond(
                    tiny_http::Response::from_string("unauthorized").with_status_code(401),
                );
                continue;
            }

            let (status, content_type, payload) = if method == tiny_http::Method::Get
                && url.starts_with("/models")
            {
                (200, "application/json", MODELS_FIXTURE.to_string())
            } else if method == tiny_http::Method::Post && url.starts_with("/chat/completions") {
                if body.contains("zai-glm-5-3") {
                    (200, "text/event-stream", CHAT_SSE_BLOCKS.to_string())
                } else if body.contains("\"stream\":true") {
                    if body.contains("\"tools\"") {
                        (200, "text/event-stream", CHAT_SSE_TOOLS.to_string())
                    } else {
                        (200, "text/event-stream", CHAT_SSE_TEXT.to_string())
                    }
                } else if body.contains("\"tools\"") {
                    (200, "application/json", CHAT_JSON_TOOLS.to_string())
                } else {
                    (200, "application/json", CHAT_JSON_TEXT.to_string())
                }
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
    MockUpstream { port }
}

struct Proxy {
    child: Option<Child>,
    port: u16,
    info_path: std::path::PathBuf,
    tmp: std::path::PathBuf,
}

/// Boot the proxy keyless in secret-push mode with an ephemeral boot token.
/// Returns the proxy plus the token so tests can drive the provisioning flow.
fn boot_proxy_secret_push(upstream_port: u16) -> (Proxy, String) {
    let tmp =
        std::env::temp_dir().join(format!("mistral-e2e-sp-{}-{}", process::id(), free_port()));
    // Remove any stale dir left by a crashed earlier run (pid reuse).
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let info = tmp.join("server.json");
    let token = "boot-token-0123456789abcdef";
    // The token travels via a file, never argv: `ps` exposes command lines.
    let token_path = tmp.join("boot-token");
    std::fs::write(&token_path, token).unwrap();

    let child = process::Command::new(env!("CARGO_BIN_EXE_codex-mistral-proxy"))
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
        .env_remove("MISTRAL_API_KEY")
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
        match http(&format!("http://127.0.0.1:{port}/health"), "GET", None) {
            Ok((status, _)) if status == 200 => break,
            _ if Instant::now() >= deadline => panic!("proxy never became healthy within 30s"),
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

/// Boot the proxy with the key on stdin.
fn boot_proxy(upstream_port: u16) -> Proxy {
    let tmp = std::env::temp_dir().join(format!("mistral-e2e-{}-{}", process::id(), free_port()));
    std::fs::create_dir_all(&tmp).unwrap();
    let info = tmp.join("server.json");

    let mut child = process::Command::new(env!("CARGO_BIN_EXE_codex-mistral-proxy"))
        .args([
            "--http-shutdown",
            "--server-info",
            info.to_str().unwrap(),
            "--upstream-base",
            &format!("http://127.0.0.1:{upstream_port}"),
        ])
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
        match http(&format!("http://127.0.0.1:{port}/health"), "GET", None) {
            Ok((status, _body)) if status == 200 => break,
            _ if Instant::now() >= deadline => panic!("proxy never became healthy within 30s"),
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
/// EOF so streaming bodies work without any extra dependency.
fn http(url: &str, method: &str, body: Option<&str>) -> Result<(u16, String), String> {
    http_ex(url, method, body, "")
}

/// `extra_headers` is appended verbatim (each entry must already end with
/// `\r\n`).
fn http_ex(
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
    let proxy = boot_proxy(mock.port);
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
    )
    .unwrap();
    assert_eq!(status, 503, "pre-provisioning must hold requests");
    assert!(body.contains("proxy_secret_pending"), "body: {body}");

    // Provisioning without the boot token is rejected.
    let (status, _body) = http_ex(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"secret-push","key_id":"primary","material":"test-key-abc123"}"#),
        "",
    )
    .unwrap();
    assert_eq!(status, 401, "missing boot token must be rejected");

    // Valid provisioning returns 204 and unblocks upstream forwarding.
    let (status, _body) = http_ex(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"secret-push","key_id":"primary","material":"test-key-abc123"}"#),
        "X-Router-Boot-Token: boot-token-0123456789abcdef\r\n",
    )
    .unwrap();
    assert_eq!(status, 204, "valid provisioning must return 204");

    // Replay is rejected.
    let (status, _body) = http_ex(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"secret-push","key_id":"primary","material":"another"}"#),
        "X-Router-Boot-Token: boot-token-0123456789abcdef\r\n",
    )
    .unwrap();
    assert_eq!(status, 409, "replay must be rejected");

    // After provisioning, upstream forwarding works with the pushed key.
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/v1/models", proxy.port),
        "GET",
        None,
    )
    .unwrap();
    assert_eq!(status, 200, "post-provisioning must forward");
}

#[test]
fn secret_push_rejects_mismatched_channel_payload() {
    let mock = start_mock();
    let (proxy, _token) = boot_proxy_secret_push(mock.port);

    let (status, body) = http_ex(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"workload-identity","key_id":"primary","material":"test-key-abc123"}"#),
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
    )
    .unwrap();
    assert_eq!(status, 503, "rejected push must leave the proxy keyless");
    assert!(body.contains("proxy_secret_pending"), "body: {body}");
}

#[test]
fn secret_push_trims_padded_material() {
    let mock = start_mock();
    let (proxy, _token) = boot_proxy_secret_push(mock.port);

    let (status, _body) = http_ex(
        &format!("http://127.0.0.1:{}/protocol/v1/secrets", proxy.port),
        "POST",
        Some(r#"{"channel":"secret-push","key_id":"primary","material":"  test-key-abc123  "}"#),
        "X-Router-Boot-Token: boot-token-0123456789abcdef\r\n",
    )
    .unwrap();
    assert_eq!(status, 204, "padded material must be trimmed and accepted");

    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/v1/models", proxy.port),
        "GET",
        None,
    )
    .unwrap();
    assert_eq!(status, 200, "the trimmed key must authenticate upstream");
}

#[test]
fn workload_identity_channel_is_rejected_at_startup() {
    let output = process::Command::new(env!("CARGO_BIN_EXE_codex-mistral-proxy"))
        .args(["--secret-channel", "workload-identity"])
        .env_remove("MISTRAL_API_KEY")
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
fn health_reports_upstream() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port);
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/health", proxy.port),
        "GET",
        None,
    )
    .unwrap();
    assert_eq!(status, 200);
    assert!(body.contains("\"status\":\"ok\""), "body: {body}");
    assert!(
        body.contains(&format!("http://127.0.0.1:{}", mock.port)),
        "body: {body}"
    );
}

#[test]
fn models_with_query_translates_to_codex_shape() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port);
    // The query is not optional: codex's ModelsClient always appends it, and
    // exact-match routing that ignores the query historically 403'd here.
    let (status, body) = http(
        &format!(
            "http://127.0.0.1:{}/v1/models?client_version=0.121.0",
            proxy.port
        ),
        "GET",
        None,
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
    assert!(
        !body.contains("mistral-embed"),
        "embed model leaked: {body}"
    );
}

#[test]
fn nonstream_chat_translates_responses_api() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port);
    let req = r#"{"model":"mistral-medium-latest","stream":false,"input":[{"type":"message","role":"user","content":"say OK"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
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
}

#[test]
fn stream_chat_emits_full_sse_sequence() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port);
    let req = r#"{"model":"mistral-medium-latest","stream":true,"input":[{"type":"message","role":"user","content":"count"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
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
    // Terminal finish_reason "stop" must complete, not mark incomplete.
    assert!(!body.contains("\"status\":\"incomplete\""), "body: {body}");
    assert!(body.contains("\"status\":\"completed\""), "body: {body}");
}

#[test]
fn tool_calls_roundtrip_both_modes() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port);

    let req = r#"{"model":"mistral-medium-latest","stream":false,"tools":[{"type":"function","name":"get_weather","description":"w","parameters":{"type":"object"}}],"input":[{"type":"message","role":"user","content":"weather?"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("\"type\":\"function_call\""), "body: {body}");
    assert!(body.contains("\"name\":\"get_weather\""), "body: {body}");
    assert!(body.contains("\"call_id\":\"call_xyz\""), "body: {body}");

    let req = r#"{"model":"mistral-medium-latest","stream":true,"tools":[{"type":"function","name":"get_weather","description":"w","parameters":{"type":"object"}}],"input":[{"type":"message","role":"user","content":"weather?"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
    )
    .unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(
        body.contains("event: response.output_item.added"),
        "body: {body}"
    );
    assert!(
        body.contains("event: response.function_call_arguments.delta"),
        "body: {body}"
    );
    assert!(
        body.contains("event: response.function_call_arguments.done"),
        "body: {body}"
    );
    assert!(body.contains("\"name\":\"get_weather\""), "body: {body}");
    assert!(body.contains("\"call_id\":\"call_xyz\""), "body: {body}");
    assert!(body.contains("event: response.completed"), "body: {body}");
}

#[test]
fn stream_reasoning_blocks_surface_text_through_proxied_sse() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port);
    let req = r#"{"model":"zai-glm-5-3","stream":true,"input":[{"type":"message","role":"user","content":"ping"}]}"#;
    let (status, body) = http(
        &format!("http://127.0.0.1:{}/v1/responses", proxy.port),
        "POST",
        Some(req),
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
fn unknown_route_is_403() {
    let mock = start_mock();
    let proxy = boot_proxy(mock.port);
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/nope", proxy.port),
        "GET",
        None,
    )
    .unwrap();
    assert_eq!(status, 403);
}

#[test]
fn shutdown_endpoint_terminates_process() {
    let mock = start_mock();
    let mut proxy = boot_proxy(mock.port);
    let (status, _body) = http(
        &format!("http://127.0.0.1:{}/shutdown", proxy.port),
        "GET",
        None,
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
    drop(proxy); // Drop impl cleans tmp; child already taken.
}

// `free_port` is used by boot_proxy via placeholder naming; silence unused with a real use.
#[allow(dead_code)]
fn _uses() -> u16 {
    free_port()
}
