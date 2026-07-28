/// The `codex exec` design loop: repeatedly ask a model to write an
/// AssemblyScript scenario until `asc --noEmit` accepts it, or give up
/// after a bounded number of attempts. See `README.md` — the `tb-design`
/// row — and `skills/threadbox-designer.md` for what the model is told.
/// Zero dependencies beyond std, per `AGENTS.md`.
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Args {
    scenario: String,
    guest_dir: PathBuf,
    skill_path: PathBuf,
    log_dir: PathBuf,
    max_attempts: u32,
    codex_bin: String,
}

fn main() {
    let raw: Vec<String> = std::env::args().collect();
    let args = match parse_args(&raw[1..]) {
        Ok(a) => a,
        Err(message) => {
            eprintln!("{message}");
            eprintln!("usage: tb-design --scenario <name> [--guest-dir <path>] [--skill <path>] [--log-dir <path>] [--max-attempts <n>]");
            std::process::exit(1);
        }
    };
    std::process::exit(run(&args));
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut scenario: Option<String> = None;
    let mut guest_dir = PathBuf::from("../guest");
    let mut skill_path = PathBuf::from("../skills/threadbox-designer.md");
    let mut log_dir = PathBuf::from("logs");
    let mut max_attempts: u32 = 5;
    let mut codex_bin = "codex".to_string();

    let mut i = 0;
    while i < raw.len() {
        match raw[i].as_str() {
            "--scenario" => {
                scenario = Some(next_value(raw, &mut i, "--scenario")?);
            }
            "--guest-dir" => {
                guest_dir = PathBuf::from(next_value(raw, &mut i, "--guest-dir")?);
            }
            "--skill" => {
                skill_path = PathBuf::from(next_value(raw, &mut i, "--skill")?);
            }
            "--log-dir" => {
                log_dir = PathBuf::from(next_value(raw, &mut i, "--log-dir")?);
            }
            "--max-attempts" => {
                let value = next_value(raw, &mut i, "--max-attempts")?;
                max_attempts = value
                    .parse()
                    .map_err(|_| format!("--max-attempts value \"{value}\" is not a positive integer"))?;
            }
            "--codex-bin" => {
                codex_bin = next_value(raw, &mut i, "--codex-bin")?;
            }
            other => return Err(format!("unrecognized argument \"{other}\"")),
        }
    }

    let scenario = scenario.ok_or_else(|| "missing required argument --scenario".to_string())?;
    Ok(Args { scenario, guest_dir, skill_path, log_dir, max_attempts, codex_bin })
}

fn next_value(raw: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    let value = raw.get(*i + 1).cloned().ok_or_else(|| format!("{flag} requires a value"))?;
    *i += 2;
    Ok(value)
}

fn run(args: &Args) -> i32 {
    let target_ts = args.guest_dir.join("examples").join(format!("{}.ts", args.scenario));
    let scenario_log_dir = args.log_dir.join(&args.scenario);
    if let Err(e) = fs::create_dir_all(&scenario_log_dir) {
        eprintln!("tb-design: cannot create log directory \"{}\": {e}", scenario_log_dir.display());
        return 1;
    }
    let log_path = scenario_log_dir.join("attempts.jsonl");

    let mut prev_errors: Option<String> = None;

    for attempt in 1..=args.max_attempts {
        let prompt = build_prompt(&args.scenario, attempt, prev_errors.as_deref());

        let codex_result = Command::new(&args.codex_bin)
            .arg("exec")
            .arg("--skill")
            .arg(&args.skill_path)
            .arg(&prompt)
            .env("TB_DESIGN_TARGET", &target_ts)
            .env("TB_DESIGN_ATTEMPT", attempt.to_string())
            .output();

        let codex_output = match codex_result {
            Ok(output) => output,
            Err(e) => {
                append_log(&log_path, attempt, false, &format!("failed to run \"{}\": {e}", args.codex_bin));
                eprintln!("tb-design: attempt {attempt} could not run \"{}\": {e}", args.codex_bin);
                prev_errors = Some(format!("could not run codex: {e}"));
                continue;
            }
        };

        if !target_ts.exists() {
            let message = format!(
                "codex exec exited {} but did not write \"{}\"",
                codex_output.status,
                target_ts.display()
            );
            append_log(&log_path, attempt, false, &message);
            prev_errors = Some(message);
            continue;
        }

        match check_type_checks(&args.guest_dir, &target_ts) {
            Ok(()) => {
                append_log(&log_path, attempt, true, "");
                eprintln!("tb-design: scenario \"{}\" converged after {attempt} attempt(s)", args.scenario);
                return 0;
            }
            Err(errors) => {
                append_log(&log_path, attempt, false, &errors);
                prev_errors = Some(errors);
            }
        }
    }

    eprintln!(
        "tb-design: scenario \"{}\" did not converge after {} attempt(s)",
        args.scenario, args.max_attempts
    );
    1
}

/// Build the prompt handed to `codex exec` on this attempt. Attempt 1
/// asks for a fresh program; later attempts include the prior
/// `asc --noEmit` diagnostics so the model can fix them.
fn build_prompt(scenario: &str, attempt: u32, prev_errors: Option<&str>) -> String {
    match prev_errors {
        None => format!(
            "Write a ThreadBox scenario named \"{scenario}\" using the DSL described in your skill. Write the complete program to the path named by the TB_DESIGN_TARGET environment variable."
        ),
        Some(errors) => format!(
            "Attempt {attempt}: the program you wrote for scenario \"{scenario}\" failed asc --noEmit with:\n{errors}\nFix the program at the path named by the TB_DESIGN_TARGET environment variable. Do not change any other file."
        ),
    }
}

/// Run `asc --noEmit` against `target_ts` from within `guest_dir`, using
/// the compiler installed at `guest_dir/node_modules/.bin/asc` directly
/// rather than through `npx`, so behavior does not depend on `npx`'s own
/// directory-walking resolution.
fn check_type_checks(guest_dir: &Path, target_ts: &Path) -> Result<(), String> {
    let asc_bin = guest_dir.join("node_modules/.bin/asc");
    let relative = target_ts
        .strip_prefix(guest_dir)
        .map_err(|_| format!("\"{}\" is not inside guest dir \"{}\"", target_ts.display(), guest_dir.display()))?;

    let output = Command::new(&asc_bin)
        .arg(relative.to_string_lossy().as_ref())
        .arg("--noEmit")
        .current_dir(guest_dir)
        .output()
        .map_err(|e| format!("failed to run \"{}\": {e}", asc_bin.display()))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

/// Append one JSON line to `log_path`, opening in append mode so the
/// log is never truncated, per `AGENTS.md`. Hand-rolled JSON (no
/// serialization crate), matching this crate's zero-dependency rule.
fn append_log(log_path: &Path, attempt: u32, success: bool, errors: &str) {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = format!(
        "{{\"attempt\":{attempt},\"success\":{success},\"ts\":{ts},\"errors\":{}}}\n",
        json_escape(errors)
    );
    let opened = OpenOptions::new().create(true).append(true).open(log_path);
    match opened {
        Ok(mut file) => {
            if let Err(e) = file.write_all(line.as_bytes()) {
                eprintln!("tb-design: cannot append to log \"{}\": {e}", log_path.display());
            }
        }
        Err(e) => eprintln!("tb-design: cannot open log \"{}\": {e}", log_path.display()),
    }
}

fn json_escape(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
