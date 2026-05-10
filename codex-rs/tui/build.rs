use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    emit_git_rerun_directives();

    let version = exact_tag()
        .or_else(derived_version)
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    println!("cargo:rustc-env=CODEX_CLI_VERSION={version}");
}

fn emit_git_rerun_directives() {
    let Some(git_dir) = git_output(["rev-parse", "--git-dir"]) else {
        return;
    };

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string()));
    let git_dir = PathBuf::from(git_dir);
    let git_dir = if git_dir.is_absolute() {
        git_dir
    } else {
        manifest_dir.join(git_dir)
    };

    for path in [
        git_dir.join("HEAD"),
        git_dir.join("index"),
        git_dir.join("packed-refs"),
        git_dir.join("refs"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn exact_tag() -> Option<String> {
    git_output(["describe", "--tags", "--exact-match"])
}

fn derived_version() -> Option<String> {
    let date = git_output([
        "show",
        "-s",
        "--date=format-local:%Y.%m.%d",
        "--format=%cd",
        "HEAD",
    ])?;
    let sha = git_output(["rev-parse", "--short=10", "HEAD"])?;
    let dirty_suffix = if git_output(["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|s| !s.is_empty())
    {
        "-dirty"
    } else {
        ""
    };

    Some(format!("{date}-{sha}{dirty_suffix}"))
}

fn git_output<const N: usize>(args: [&str; N]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}
