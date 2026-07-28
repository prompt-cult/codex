//! Validate an IR document from the command line, using the same reader and
//! the same validators the runtime uses. A graph that fails here never runs.
fn main() {
    let path = std::env::args().nth(1).expect("usage: validate <ir.json>");
    let source = std::fs::read_to_string(&path).expect("cannot read IR");
    match threadbox_evaluator::ir::parse(&source) {
        Ok(graph) => match threadbox_evaluator::ir::validate(&graph) {
            Ok(()) => println!(
                "valid: {} v{} ({} top-level nodes)",
                graph.id,
                graph.version,
                graph.nodes.len()
            ),
            Err(e) => {
                eprintln!("invalid: {e}");
                std::process::exit(1);
            }
        },
        Err(e) => {
            eprintln!("unreadable: {e}");
            std::process::exit(1);
        }
    }
}
