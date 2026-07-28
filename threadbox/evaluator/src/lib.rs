//! ThreadBox evaluator.
//!
//! One crate, two consumers: compiled to `wasm32-unknown-unknown` it is the
//! runtime loaded by a browser extension; compiled natively it is exercised by
//! `cargo test`. Both run the same code, so a CLI test is evidence about the
//! browser runtime rather than a separate thing that resembles it.
//!
//! The evaluator is pure. It has no clock, no randomness, no filesystem, no
//! network, and no browser API. Everything it needs, it asks the host for
//! through the envelope in `ABI.md`.

pub mod generated;
pub mod ir;
pub mod query;
pub mod run;

#[cfg(target_arch = "wasm32")]
mod wasm;

pub use ir::{Graph, IrError};
pub use run::{Config, EvalError, Evaluator};

/// Load and validate a graph, then build an evaluator for it. A graph that
/// fails any validator is never run.
pub fn load(ir_source: &str, config_source: &str) -> Result<Evaluator, String> {
    let graph = ir::parse(ir_source).map_err(|e| e.message)?;
    ir::validate(&graph).map_err(|e| e.message)?;
    let config = run::Config::from_json(config_source).map_err(|e| e.message)?;
    Ok(Evaluator::new(graph, config))
}
