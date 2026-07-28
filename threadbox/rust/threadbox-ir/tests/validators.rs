use threadbox_ir::{parse, validate};

fn ok_wrap(json: &str) -> String {
    format!("{{\"ir\":\"threadbox.ir.v1\",\"nodes\":{json}}}")
}

#[test]
fn zero_publish() {
    let src = ok_wrap("[{\"i\":0,\"kind\":\"Screenshot\",\"parents\":[]}]");
    let graph = parse(&src).expect("should parse");
    let err = validate(&graph).unwrap_err();
    assert_eq!(err.to_string(), "graph has 0 Publish nodes; exactly one is required");
}

#[test]
fn two_publish_nodes() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":1,\"kind\":\"Publish\",\"parents\":[0]},\
          {\"i\":2,\"kind\":\"Publish\",\"parents\":[1]}]",
    );
    let graph = parse(&src).expect("should parse");
    let err = validate(&graph).unwrap_err();
    assert_eq!(
        err.to_string(),
        "graph has 2 Publish nodes at indices [1, 2]; exactly one is required"
    );
}

#[test]
fn orphan_node_is_rejected() {
    // Node 0 (Screenshot) is built but never consumed; Publish only
    // reaches node 1 (LoadJson).
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":1,\"kind\":\"LoadJson\",\"parents\":[],\"name\":\"submission\"},\
          {\"i\":2,\"kind\":\"Publish\",\"parents\":[1]}]",
    );
    let graph = parse(&src).expect("should parse");
    let err = validate(&graph).unwrap_err();
    assert_eq!(
        err.to_string(),
        "node 0 (Screenshot) is unreachable from the Publish at index 2"
    );
}

#[test]
fn retry_bound_zero_is_rejected() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":1,\"kind\":\"Retry\",\"parents\":[0],\"bound\":0},\
          {\"i\":2,\"kind\":\"Publish\",\"parents\":[1]}]",
    );
    let graph = parse(&src).expect("should parse");
    let err = validate(&graph).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Retry at index 1 has bound 0; bound must be a positive integer literal"
    );
}

#[test]
fn retry_bound_negative_is_rejected() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":1,\"kind\":\"Retry\",\"parents\":[0],\"bound\":-1},\
          {\"i\":2,\"kind\":\"Publish\",\"parents\":[1]}]",
    );
    let graph = parse(&src).expect("should parse");
    let err = validate(&graph).unwrap_err();
    assert_eq!(
        err.to_string(),
        "Retry at index 1 has bound -1; bound must be a positive integer literal"
    );
}

#[test]
fn foreach_body_not_below_is_rejected() {
    // The reader's bounds check for `body` is against the *whole* arena
    // (per IR.md: "the reader checks only that the index exists"), so a
    // `body` pointing forward of its own `ForEach` — but still within
    // the arena — passes the reader and must be caught by validator 4.
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"ForEach\",\"parents\":[],\"over\":\"answers\",\"body\":1},\
          {\"i\":1,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":2,\"kind\":\"Publish\",\"parents\":[0]}]",
    );
    let graph = parse(&src).expect("forward body index is within bounds, reader must accept it");
    let err = validate(&graph).unwrap_err();
    assert_eq!(
        err.to_string(),
        "ForEach at index 0 names body index 1, which is not below it; a body is built before the ForEach that names it"
    );
}

#[test]
fn foreach_body_out_of_bounds_is_rejected_by_reader() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"ForEach\",\"parents\":[],\"over\":\"answers\",\"body\":5},\
          {\"i\":1,\"kind\":\"Publish\",\"parents\":[0]}]",
    );
    let err = parse(&src).unwrap_err();
    assert_eq!(
        err.to_string(),
        "ForEach at index 0 references body index 5 but the arena holds 2 nodes"
    );
}

#[test]
fn validator_ordering_publish_error_wins_over_orphan() {
    // Zero Publish nodes AND an orphan both present; validator 1 must
    // fire first, per IR.md's fixed validator order.
    let src = ok_wrap("[{\"i\":0,\"kind\":\"Screenshot\",\"parents\":[]}]");
    let graph = parse(&src).expect("should parse");
    let err = validate(&graph).unwrap_err();
    assert_eq!(err.to_string(), "graph has 0 Publish nodes; exactly one is required");
}

/// The exact worked fragment from `IR.md` — a document, a screenshot, a
/// scale, a locate, a click, a field-driven type, a retry bound, and the
/// terminal — must parse and pass all four validators.
#[test]
fn ir_md_worked_fragment_is_valid() {
    let src = ok_wrap(
        "[\
          {\"i\":0,\"kind\":\"LoadJson\",\"parents\":[],\"name\":\"submission\"},\
          {\"i\":1,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":2,\"kind\":\"Scale\",\"parents\":[1],\"width\":1280},\
          {\"i\":3,\"kind\":\"Locate\",\"parents\":[2],\
            \"description\":\"the input labelled 'Case Number'\",\
            \"model\":{\"vendor\":\"anthropic\",\"model\":\"opus-performance\",\
                       \"think\":\"medium\",\"contextWindow\":1000000}},\
          {\"i\":4,\"kind\":\"Click\",\"parents\":[3]},\
          {\"i\":5,\"kind\":\"Type\",\"parents\":[4,0],\
            \"valueKind\":\"field\",\"value\":\"caseNumber\"},\
          {\"i\":6,\"kind\":\"Retry\",\"parents\":[5],\"bound\":3},\
          {\"i\":7,\"kind\":\"Publish\",\"parents\":[6]}\
        ]",
    );
    let graph = parse(&src).expect("worked fragment should parse");
    assert_eq!(graph.nodes.len(), 8);
    validate(&graph).expect("worked fragment should pass all four validators");
}
