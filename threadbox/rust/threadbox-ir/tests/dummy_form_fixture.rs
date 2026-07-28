/// Dogfoods `threadbox-ir` against the hand-authored IR fixture used for
/// the end-to-end CDP replay proof in `harness/dummy-form/`. If this
/// fixture ever fails to parse or validate, the e2e proof's premise —
/// that a real IR graph, not a hardcoded script, drove the replay — no
/// longer holds.
use std::fs;
use threadbox_ir::{parse, validate};

#[test]
fn crm_search_edit_fixture_is_valid() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let path = format!("{manifest_dir}/../../harness/dummy-form/fixtures/crm-search-edit.ir.json");
    let json = fs::read_to_string(&path).expect("crm-search-edit.ir.json should exist");

    let graph = parse(&json).expect("fixture should parse");
    validate(&graph).expect("fixture should pass all four validators");

    assert_eq!(graph.nodes.len(), 24);
    let publish_count = graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, threadbox_ir::NodeKind::Publish))
        .count();
    assert_eq!(publish_count, 1);
}
