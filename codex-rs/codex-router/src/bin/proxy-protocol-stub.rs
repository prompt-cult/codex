//! Test-support stub proxy speaking Prompt Cult Proxy Protocol v1.
//!
//! Used by `tests/secret_flow.rs` to boot the real `codex-proxy-router`
//! against controlled children. The stub advertises the capability list from
//! a sibling `<binary>.channels.json` file, publishes a v1 `server-info`,
//! serves the secrets endpoint (recording each delivery as
//! `<binary>.record.json` next to its own executable), and answers every
//! other request with `200 ok`. It also records which provider API key
//! environment variables leaked into its environment, so tests can assert
//! the router's env sanitization.

use std::io::Write;

use codex_proxy_protocol::protocol::PROTOCOL_VERSION;
use codex_proxy_protocol::protocol::SecretChannelList;
use codex_proxy_protocol::protocol::ServerInfoV1;

/// Provider API key variables whose presence in the stub's environment must
/// be recorded for the tests to assert on.
const SECRET_ENV_NAMES: &[&str] = &["OPENCODE_API_KEY", "MISTRAL_API_KEY"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut port: u16 = 0;
    let mut info_path = String::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                i += 1;
                port = args[i].parse().unwrap();
            }
            "--server-info" => {
                i += 1;
                info_path = args[i].clone();
            }
            _ => {}
        }
        i += 1;
    }

    let exe = std::env::current_exe().unwrap();
    let exe_dir = exe.parent().unwrap().to_path_buf();
    let exe_name = exe.file_name().unwrap().to_string_lossy().to_string();
    let channels: Vec<String> = serde_json::from_str(
        &std::fs::read_to_string(exe_dir.join(format!("{exe_name}.channels.json"))).unwrap(),
    )
    .unwrap();
    let secret_channels: Vec<codex_proxy_protocol::protocol::SecretChannel> = channels
        .iter()
        .map(|name| serde_json::from_value(serde_json::Value::String(name.clone())).unwrap())
        .collect();
    // Optional advertised-protocol-version override, used by the mismatch
    // tests to simulate a child speaking a different protocol major.
    let protocol_version =
        std::fs::read_to_string(exe_dir.join(format!("{exe_name}.protocol-version")))
            .ok()
            .and_then(|text| text.trim().parse::<u32>().ok())
            .unwrap_or(PROTOCOL_VERSION);

    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    let port = listener.local_addr().unwrap().port();
    if !info_path.is_empty() {
        let info = ServerInfoV1 {
            server_info_version: 1,
            port,
            pid: std::process::id(),
            proxy_kind: "stub".to_string(),
            protocol_version,
            secret_channels: SecretChannelList(secret_channels),
        };
        info.write_to_file(std::path::Path::new(&info_path))
            .unwrap();
    }

    let env_leaks: Vec<String> = SECRET_ENV_NAMES
        .iter()
        .filter(|name| {
            std::env::var(name)
                .map(|value| !value.is_empty())
                .unwrap_or(false)
        })
        .map(|name| (*name).to_string())
        .collect();

    // Environment snapshot at boot, for tests to assert the router's env
    // sanitization (allow-list in env-debug, keyless in production modes).
    let snapshot = serde_json::json!({ "env_leaks": env_leaks });
    std::fs::write(
        exe_dir.join(format!("{exe_name}.env-snapshot.json")),
        serde_json::to_vec(&snapshot).unwrap(),
    )
    .ok();

    for stream in listener.incoming() {
        let mut stream = stream.unwrap();
        // Read headers (and any body) without waiting for EOF: callers may
        // hold the connection open for keep-alive.
        let mut raw = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match std::io::Read::read(&mut stream, &mut byte) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    raw.push(byte[0]);
                    let n = raw.len();
                    if n >= 4 && &raw[n - 4..] == b"\r\n\r\n" {
                        break;
                    }
                }
            }
        }
        let headers = String::from_utf8_lossy(&raw).to_string();
        let content_length: usize = headers
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
            .and_then(|l| l.split(':').nth(1))
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; content_length];
        if content_length > 0 {
            let _ = std::io::Read::read_exact(&mut stream, &mut body);
        }
        if headers.contains(codex_proxy_protocol::protocol::SECRETS_ENDPOINT) {
            let record = serde_json::json!({
                "headers": headers,
                "body": String::from_utf8_lossy(&body).to_string(),
                "env_leaks": env_leaks,
            });
            std::fs::write(
                exe_dir.join(format!("{exe_name}.record.json")),
                serde_json::to_vec(&record).unwrap(),
            )
            .ok();
            stream
                .write_all(b"HTTP/1.1 204 No Content\r\ncontent-length: 0\r\n\r\n")
                .ok();
        } else {
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                .ok();
        }
    }
}
