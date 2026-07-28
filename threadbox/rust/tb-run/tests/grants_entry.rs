use std::process::Command;

/// End-to-end proof for `tb-run`: compile `guest/examples/grants-entry.ts`
/// via `asc`, run it under `wasmi`, capture the emitted graph, validate
/// it, and check structural facts about the printed JSON. Requires `asc`
/// on PATH inside `guest/` (i.e. `guest/node_modules/assemblyscript`
/// installed) — `#[ignore]` by default so a plain `cargo test` does not
/// require that toolchain; run with `cargo test -- --include-ignored`
/// for the full check, per `AGENTS.md`'s verification commands table.
#[test]
#[ignore = "requires asc on PATH under guest/; run with --include-ignored"]
fn grants_entry_end_to_end() {
    let tb_run = env!("CARGO_BIN_EXE_tb-run");
    let grants_ts = concat!(env!("CARGO_MANIFEST_DIR"), "/../../guest/examples/grants-entry.ts");

    let output = Command::new(tb_run)
        .arg(grants_ts)
        .output()
        .expect("failed to spawn tb-run");

    assert!(
        output.status.success(),
        "tb-run failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("tb-run stdout is not UTF-8");

    assert!(stdout.contains("\"ir\":\"threadbox.ir.v1\""), "missing ir envelope tag; got: {stdout}");
    assert_eq!(
        stdout.matches("\"kind\":\"Publish\"").count(),
        1,
        "expected exactly one Publish node; got: {stdout}"
    );
    assert!(stdout.contains("\"name\":\"submission\""), "missing LoadJson(\"submission\")");
    assert!(stdout.contains("\"kind\":\"ForEach\""), "missing ForEach (Multi.over(...).forEach(...))");
    assert!(stdout.contains("\"kind\":\"Branch\""), "missing Branch (.branch(...))");
    assert!(stdout.contains("\"kind\":\"Fallback\""), "missing Fallback (.orElse(...))");
    assert!(stdout.contains("\"over\":\"answers\""), "missing ForEach over \"answers\"");

    let node_count = stdout.matches("\"kind\":").count();
    assert!(node_count >= 20, "expected at least 20 nodes in the grants-entry graph, got {node_count}");
}
