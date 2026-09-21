//! Entry point for the `codex-proxy-router` routing dispatcher; see `README.proxy-router.md`.

use clap::Parser;
use codex_router::Args;

#[ctor::ctor]
fn pre_main() {
    codex_process_hardening::pre_main_hardening();
}

pub fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    codex_router::run_main(args)
}
