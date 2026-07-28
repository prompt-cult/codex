use std::fs;
use std::path::PathBuf;
use std::process::Command;

/// CI-safe smoke test for the `codex exec` design loop: a fake `codex`
/// binary (see `tests/fixtures/fake-codex`) writes a program with a type
/// error on attempt 1 and a valid program from attempt 2 onward, driven
/// entirely by the `TB_DESIGN_TARGET` / `TB_DESIGN_ATTEMPT` environment
/// variables `tb-design` sets. This proves the loop's convergence and
/// logging behavior without any live model access. See `README.md` for
/// the manual smoke-test command against a real `codex` binary.
#[test]
#[ignore = "requires guest/node_modules/assemblyscript from the sibling guest/ directory; run with --include-ignored"]
fn design_loop_converges_with_fake_codex() {
    let tb_design = env!("CARGO_BIN_EXE_tb-design");
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let real_guest = manifest_dir.join("../../guest");
    let fake_codex = manifest_dir.join("tests/fixtures/fake-codex");

    let tmp = tempdir();
    let guest_dir = tmp.join("guest");
    fs::create_dir_all(guest_dir.join("examples")).unwrap();
    // Symlink the real node_modules so `asc` resolves without a real
    // network install; this test only needs `--noEmit` type-checking of
    // a program with no imports, so no other guest file is needed.
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        real_guest.join("node_modules"),
        guest_dir.join("node_modules"),
    )
    .unwrap();

    let log_dir = tmp.join("logs");

    // Prepend a directory containing `fake-codex` as `codex` onto PATH.
    let path_dir = tmp.join("bin");
    fs::create_dir_all(&path_dir).unwrap();
    let codex_link = path_dir.join("codex");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&fake_codex, &codex_link).unwrap();
    let old_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{}:{old_path}", path_dir.display());

    let output = Command::new(tb_design)
        .args([
            "--scenario",
            "smoke-test",
            "--guest-dir",
            guest_dir.to_str().unwrap(),
            "--log-dir",
            log_dir.to_str().unwrap(),
            "--max-attempts",
            "5",
        ])
        .env("PATH", &new_path)
        .output()
        .expect("failed to spawn tb-design");

    assert!(
        output.status.success(),
        "tb-design did not converge:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let target_ts = guest_dir.join("examples/smoke-test.ts");
    assert!(target_ts.exists(), "tb-design did not leave the converged program on disk");

    let log_path = log_dir.join("smoke-test/attempts.jsonl");
    let log_contents = fs::read_to_string(&log_path).expect("attempts.jsonl should exist");
    let lines: Vec<&str> = log_contents.lines().collect();
    assert!(
        lines.len() >= 2,
        "expected at least 2 attempt log lines (one failure, one success), got {}: {log_contents}",
        lines.len()
    );
    assert!(lines[0].contains("\"success\":false"), "attempt 1 should have failed: {}", lines[0]);
    assert!(
        lines.last().unwrap().contains("\"success\":true"),
        "final attempt should have succeeded: {}",
        lines.last().unwrap()
    );
}

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tb-design-smoke-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
