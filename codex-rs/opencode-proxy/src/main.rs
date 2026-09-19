use clap::Parser;
use codex_opencode_proxy::Args;

#[ctor::ctor]
fn pre_main() {
    codex_process_hardening::pre_main_hardening();
}

pub fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    codex_opencode_proxy::run_main(args)
}
