//! End-to-end breakage oracle for the zen proxy.
//!
//! Boots the real `codex-zen-proxy` binary against a local mock upstream and
//! drives: health, GPT passthrough (byte-for-byte) and Claude translation,
//! auth injection, route guarding, and clean shutdown.

use std::io::prelude::*;
use std::net::TcpListener;
use std::net::TcpStream;
use std::process;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;
use std::time::Instant;

const KEY: &str = "zen-test-key-456";
const AUTH: &str = "Bearer zen-test-key-456";

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

fn start_mock() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("mock bind");
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for mut request in tiny_http::Server::from_listener(listener, None).unwrap().incoming_requests() {
            let method = request.method().clone();
            let url = request.url().to_string();
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);

            const API_KEY: &str = "zen-test-key-456";
            let authed = request
                .headers()
                .iter()
                .any(|h| {
                    (h.field.equiv("Authorization") && h.value.as_str() == AUTH)
                        || (h.field.equiv("x-api-key") && h.value.as_str() == API_KEY)
                });
            if !authed {
                let _ = request.respond(tiny_http::Response::from_string("unauthorized").with_status_code(401));
                continue;
            }

            let (status, content_type, payload) = if method == tiny_http::Method::Post && url.starts_with("/responses") {
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
    port
}

struct Proxy {
    child: Option<Child>,
    port: u16,
    tmp: std::path::PathBuf,
}

fn boot_proxy(upstream_port: u16) -> Proxy {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = std::env::temp_dir().join(format!("zen-e2e-{}-{}", process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    std::fs::create_dir_all(&tmp).unwrap();
    let info = tmp.join("server.json");

    let mut child = process::Command::new(env!("CARGO_BIN_EXE_codex-zen-proxy"))
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
    child.stdin.take().unwrap().write_all(KEY.as_bytes()).unwrap();

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
            panic!("zen proxy did not publish server-info within 30s");
        }
        thread::sleep(Duration::from_millis(100));
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match http(&format!("http://127.0.0.1:{port}/health"), "GET", None) {
            Ok((status, _)) if status == 200 => break,
            _ if Instant::now() >= deadline => panic!("zen proxy never became healthy"),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }
    Proxy {
        child: Some(child),
        port,
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

fn http(url: &str, method: &str, body: Option<&str>) -> Result<(u16, String), String> {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match rest.split_once('/') {
        Some((h, p)) => (h.to_string(), format!("/{p}")),
        None => (rest.to_string(), "/".to_string()),
    };
    let mut stream = std::net::TcpStream::connect(&host_port).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n");
    match body {
        Some(b) => {
            req.push_str(&format!("content-type: application/json\r\ncontent-length: {}\r\n\r\n", b.len()));
            stream.write_all(req.as_bytes()).ok();
            stream.write_all(b.as_bytes()).ok();
        }
        None => {
            req.push_str("\r\n");
            stream.write_all(req.as_bytes()).ok();
        }
    }
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(|e| format!("read: {e}"))?;
    let text = String::from_utf8_lossy(&raw).to_string();
    let status: u16 = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "no status line".to_string())?;
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
    Ok((status, body))
}

#[test]
fn health_reports_upstream() {
    let mock = start_mock();
    let proxy = boot_proxy(mock);
    let (status, body) = http(&format!("http://127.0.0.1:{}/health", proxy.port), "GET", None).unwrap();
    assert_eq!(status, 200);
    assert!(body.contains("\"status\":\"ok\""), "body: {body}");
}

#[test]
fn gpt_passthrough_streams_verbatim() {
    let mock = start_mock();
    let proxy = boot_proxy(mock);
    let body = r#"{"model":"gpt-5.6-luna","stream":true,"input":[{"type":"message","role":"user","content":"hi"}]}"#;
    let (status, body) = http(&format!("http://127.0.0.1:{}/v1/responses", proxy.port), "POST", Some(body)).unwrap();
    assert_eq!(status, 200, "body: {body}");
    // Passthrough: the mock's exact SSE bytes must emerge unchanged.
    assert!(body.contains("event: response.output_text.delta"), "body: {body}");
    assert!(body.contains("\"delta\":\"hi\""), "body: {body}");
    assert!(body.contains("event: response.completed"), "body: {body}");
}

#[test]
fn claude_translate_streams_to_oai_sse() {
    let mock = start_mock();
    let proxy = boot_proxy(mock);
    let body = r#"{"model":"claude-sonnet-4","stream":true,"input":[{"type":"message","role":"user","content":"hi"}]}"#;
    let (status, body) = http(&format!("http://127.0.0.1:{}/v1/responses", proxy.port), "POST", Some(body)).unwrap();
    assert_eq!(status, 200, "body: {body}");
    assert!(body.contains("event: response.created"), "body: {body}");
    assert!(body.contains("event: response.output_text.delta"), "body: {body}");
    assert!(body.contains("\"delta\":\"hello\""), "translated text missing: {body}");
    assert!(body.contains("event: response.completed"), "body: {body}");
}

#[test]
fn unknown_route_is_403() {
    let mock = start_mock();
    let proxy = boot_proxy(mock);
    let (status, _body) = http(&format!("http://127.0.0.1:{}/nope", proxy.port), "GET", None).unwrap();
    assert_eq!(status, 403);
}

#[test]
fn shutdown_endpoint_terminates_process() {
    let mock = start_mock();
    let mut proxy = boot_proxy(mock);
    let (status, _body) = http(&format!("http://127.0.0.1:{}/shutdown", proxy.port), "GET", None).unwrap();
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
                    panic!("zen proxy did not exit after /shutdown within 30s");
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
    drop(proxy);
}
