//! End-to-end breakage oracle for the routing dispatcher's secret flow.
//!
//! Boots the real `codex-proxy-router` binary against the compiled
//! `proxy-protocol-stub` children (see `src/bin/proxy-protocol-stub.rs`) and
//! drives the protocol v1 secret supply channels: `secret-push` (keys
//! delivered over authenticated loopback, never in environ) and `env-debug`
//! (allow-listed env copy, testing only).

use std::fs;
use std::io::prelude::*;
use std::net::TcpStream;
use std::process;
use std::process::Child;
use std::thread;
use std::time::Duration;
use std::time::Instant;

/// Installs a stub child proxy under the backend's binary name: copies the
/// compiled stub and writes its capability advertisement. The stub reads the
/// channel list from `<binary>.channels.json` next to its executable and
/// records secret deliveries as `<binary>.record.json`.
fn install_stub(dir: &std::path::Path, name: &str, channels: &[&str]) {
    let stub_bin = env!("CARGO_BIN_EXE_proxy-protocol-stub");
    fs::copy(stub_bin, dir.join(name)).expect("copy stub binary");
    fs::write(
        dir.join(format!("{name}.channels.json")),
        serde_json::to_string(channels).unwrap(),
    )
    .unwrap();
}

/// Boots the real router with stub children in place and returns (port, child).
fn boot_router(secret_mode: &str, stub_dir: &std::path::Path) -> (u16, Child) {
    // The router locates siblings next to its own executable, so the test
    // copies the router binary next to the installed stubs.
    let router_src = env!("CARGO_BIN_EXE_codex-proxy-router");
    let router_bin = stub_dir.join("codex-proxy-router");
    fs::copy(router_src, &router_bin).expect("copy router binary");

    let info = stub_dir.join("router-info.json");
    let child = process::Command::new(&router_bin)
        .args([
            "--secret-mode",
            secret_mode,
            "--server-info",
            info.to_str().unwrap(),
        ])
        .env("OPENCODE_API_KEY", "opencode-test-key-456")
        .env("MISTRAL_API_KEY", "test-key-abc123")
        .stdin(process::Stdio::null())
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::inherit())
        .spawn()
        .expect("spawn router");

    let deadline = Instant::now() + Duration::from_secs(30);
    let port = loop {
        if let Ok(text) = fs::read_to_string(&info) {
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
            panic!("router did not publish server-info within 30s");
        }
        thread::sleep(Duration::from_millis(100));
    };
    (port, child)
}

/// Waits until the stub's record file exists and parses, then returns it.
/// Polls with `read_to_string` (never an exists-then-read race).
fn wait_for_record(dir: &std::path::Path, name: &str) -> serde_json::Value {
    let record_path = dir.join(format!("{name}.record.json"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = fs::read_to_string(&record_path)
            && let Ok(record) = serde_json::from_str::<serde_json::Value>(&text)
        {
            return record;
        }
        if Instant::now() >= deadline {
            panic!("no secret-push delivery recorded by stub {name}");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Waits until the stub's boot environment snapshot exists and parses, then
/// returns the list of secret env var names that leaked into the child.
fn wait_for_snapshot(dir: &std::path::Path, name: &str) -> Vec<String> {
    let snapshot_path = dir.join(format!("{name}.env-snapshot.json"));
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(text) = fs::read_to_string(&snapshot_path)
            && let Ok(snapshot) = serde_json::from_str::<serde_json::Value>(&text)
            && let Some(leaks) = snapshot["env_leaks"].as_array()
        {
            return leaks
                .iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect();
        }
        if Instant::now() >= deadline {
            panic!("no boot environment snapshot recorded by stub {name}");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

/// Writes the advertised-protocol-version override file for a stub.
fn set_stub_protocol_version(dir: &std::path::Path, name: &str, version: u32) {
    fs::write(
        dir.join(format!("{name}.protocol-version")),
        version.to_string(),
    )
    .unwrap();
}

/// Minimal std-only HTTP client.
fn http(url: &str, method: &str) -> Result<(u16, String), String> {
    let rest = url.strip_prefix("http://").unwrap_or(url);
    let (host_port, path) = match rest.split_once('/') {
        Some((h, p)) => (h.to_string(), format!("/{p}")),
        None => (rest.to_string(), "/".to_string()),
    };
    let mut stream = TcpStream::connect(&host_port).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(30))).ok();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {host_port}\r\nConnection: close\r\n\r\n");
    stream
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
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
fn router_secret_push_delivers_key_and_children_boot_clean() {
    let dir = std::env::temp_dir().join(format!("router-e2e-sp-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // Stubs advertise both channels so env-debug and secret-push negotiate.
    install_stub(&dir, "codex-opencode-proxy", &["env-debug", "secret-push"]);
    install_stub(&dir, "codex-mistral-proxy", &["env-debug", "secret-push"]);

    let (port, mut router) = boot_router("secret-push", &dir);

    // Both stubs must have been booted and served; the router is healthy.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match http(&format!("http://127.0.0.1:{port}/health"), "GET") {
            Ok((200, body)) => {
                assert!(body.contains("codex-proxy-router"), "body: {body}");
                break;
            }
            _ if Instant::now() >= deadline => panic!("router never became healthy"),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }

    // Each stub child must have received its own POST to the secrets endpoint
    // with the boot token header and its own key material over loopback —
    // and its environment must contain no provider API keys.
    for (name, material) in [
        ("codex-opencode-proxy", "opencode-test-key-456"),
        ("codex-mistral-proxy", "test-key-abc123"),
    ] {
        let record = wait_for_record(&dir, name);
        let headers = record["headers"]
            .as_str()
            .unwrap_or_default()
            .to_lowercase();
        assert!(
            headers.contains("post /protocol/v1/secrets"),
            "{name} delivery must hit the secrets endpoint; got: {headers}"
        );
        assert!(
            headers.contains("x-router-boot-token:"),
            "{name} delivery must carry the boot token header; got: {headers}"
        );
        let body = record["body"].as_str().unwrap_or_default();
        assert!(
            body.contains(material),
            "{name} delivery must carry its key material; got: {body}"
        );
        assert_eq!(
            record["env_leaks"],
            serde_json::json!([]),
            "{name} must be booted keyless in secret-push mode"
        );
    }

    let _ = router.kill();
    let _ = router.wait();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn router_refuses_to_boot_children_that_cannot_supply_secrets() {
    let dir = std::env::temp_dir().join(format!("router-e2e-refuse-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // A "commercial" stub that only resolves its own credentials: it
    // advertises workload-identity only, so env-debug must fail negotiation
    // and the router must refuse to boot.
    install_stub(&dir, "codex-opencode-proxy", &["workload-identity"]);
    install_stub(&dir, "codex-mistral-proxy", &["env-debug", "secret-push"]);

    let router_src = env!("CARGO_BIN_EXE_codex-proxy-router");
    let router_bin = dir.join("codex-proxy-router");
    fs::copy(router_src, &router_bin).expect("copy router binary");

    let info = dir.join("router-info.json");
    let output = process::Command::new(&router_bin)
        .args([
            "--secret-mode",
            "env-debug",
            "--server-info",
            info.to_str().unwrap(),
        ])
        .env("OPENCODE_API_KEY", "opencode-test-key-456")
        .env("MISTRAL_API_KEY", "test-key-abc123")
        .stdin(process::Stdio::null())
        .output()
        .expect("spawn router");

    assert!(
        !output.status.success(),
        "router must refuse to boot when negotiation fails"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("env-debug") && stderr.contains("workload-identity"),
        "refusal must name both sides; stderr: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn router_env_debug_copies_allow_listed_env_only() {
    let dir = std::env::temp_dir().join(format!("router-e2e-envdbg-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    install_stub(&dir, "codex-opencode-proxy", &["env-debug", "secret-push"]);
    install_stub(&dir, "codex-mistral-proxy", &["env-debug", "secret-push"]);

    let (port, mut router) = boot_router("env-debug", &dir);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match http(&format!("http://127.0.0.1:{port}/health"), "GET") {
            Ok((200, _)) => break,
            _ if Instant::now() >= deadline => panic!("router never became healthy"),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }

    // The allow-list is per-backend: the opencode child must see exactly
    // OPENCODE_API_KEY and the mistral child exactly MISTRAL_API_KEY — any
    // other secret name must have been stripped by the router's sanitizer.
    assert_eq!(
        wait_for_snapshot(&dir, "codex-opencode-proxy"),
        vec!["OPENCODE_API_KEY".to_string()],
        "opencode child env must contain only its allow-listed secret"
    );
    assert_eq!(
        wait_for_snapshot(&dir, "codex-mistral-proxy"),
        vec!["MISTRAL_API_KEY".to_string()],
        "mistral child env must contain only its allow-listed secret"
    );

    let _ = router.kill();
    let _ = router.wait();
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn router_bails_loudly_on_protocol_version_mismatch() {
    let dir = std::env::temp_dir().join(format!("router-e2e-vrmis-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // A child advertising a future protocol major must fail the router
    // immediately instead of silently burning the boot timeout.
    install_stub(&dir, "codex-opencode-proxy", &["env-debug", "secret-push"]);
    set_stub_protocol_version(&dir, "codex-opencode-proxy", 2);
    install_stub(&dir, "codex-mistral-proxy", &["env-debug", "secret-push"]);

    let router_src = env!("CARGO_BIN_EXE_codex-proxy-router");
    let router_bin = dir.join("codex-proxy-router");
    fs::copy(router_src, &router_bin).expect("copy router binary");

    let info = dir.join("router-info.json");
    let output = process::Command::new(&router_bin)
        .args([
            "--secret-mode",
            "secret-push",
            "--server-info",
            info.to_str().unwrap(),
        ])
        .env("OPENCODE_API_KEY", "opencode-test-key-456")
        .env("MISTRAL_API_KEY", "test-key-abc123")
        .stdin(process::Stdio::null())
        .output()
        .expect("spawn router");

    assert!(
        !output.status.success(),
        "router must refuse to boot against a mismatched protocol_version"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("protocol_version") && stderr.contains("2"),
        "refusal must name the advertised version; stderr: {stderr}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn router_workload_identity_boots_keyless_children() {
    let dir = std::env::temp_dir().join(format!("router-e2e-wlid-{}", process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // "Commercial" children that resolve their own credentials: the router
    // must boot them keyless and push nothing.
    install_stub(&dir, "codex-opencode-proxy", &["workload-identity"]);
    install_stub(&dir, "codex-mistral-proxy", &["workload-identity"]);

    let (port, mut router) = boot_router("workload-identity", &dir);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match http(&format!("http://127.0.0.1:{port}/health"), "GET") {
            Ok((200, body)) => {
                assert!(body.contains("codex-proxy-router"), "body: {body}");
                break;
            }
            _ if Instant::now() >= deadline => panic!("router never became healthy"),
            _ => thread::sleep(Duration::from_millis(100)),
        }
    }

    // No secret env may reach the children in workload-identity mode, and no
    // secret-push delivery may have been attempted.
    assert_eq!(
        wait_for_snapshot(&dir, "codex-opencode-proxy"),
        Vec::<String>::new(),
        "workload-identity children must boot keyless"
    );
    assert_eq!(
        wait_for_snapshot(&dir, "codex-mistral-proxy"),
        Vec::<String>::new(),
        "workload-identity children must boot keyless"
    );
    assert!(
        !dir.join("codex-opencode-proxy.record.json").exists()
            && !dir.join("codex-mistral-proxy.record.json").exists(),
        "workload-identity mode must not push secrets"
    );

    let _ = router.kill();
    let _ = router.wait();
    let _ = fs::remove_dir_all(&dir);
}
