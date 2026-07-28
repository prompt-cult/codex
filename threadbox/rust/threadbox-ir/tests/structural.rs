use threadbox_ir::{parse, validate};

fn ok_wrap(json: &str) -> String {
    format!("{{\"ir\":\"threadbox.ir.v1\",\"nodes\":{json}}}")
}

#[test]
fn wrong_ir_tag() {
    let src = "{\"ir\":\"threadbox.ir.v2\",\"nodes\":[]}";
    let err = parse(src).unwrap_err();
    assert_eq!(
        err.to_string(),
        "ir field is \"threadbox.ir.v2\"; expected \"threadbox.ir.v1\""
    );
}

#[test]
fn missing_ir_field() {
    let src = "{\"nodes\":[]}";
    let err = parse(src).unwrap_err();
    assert!(err.to_string().contains("missing required field \"ir\""));
}

#[test]
fn index_mismatch() {
    let src = ok_wrap("[{\"i\":5,\"kind\":\"Screenshot\",\"parents\":[]}]");
    let err = parse(&src).unwrap_err();
    assert_eq!(err.to_string(), "node at position 0 has index 5; expected 0");
}

#[test]
fn parent_not_backward() {
    // Node 0 references parent 0, which is not less than its own index.
    let src = ok_wrap("[{\"i\":0,\"kind\":\"Click\",\"parents\":[0]}]");
    let err = parse(&src).unwrap_err();
    assert_eq!(
        err.to_string(),
        "node 0 (Click) has parent 0 which is not less than its own index 0; edges must point backward"
    );
}

#[test]
fn parent_out_of_bounds_forward() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":1,\"kind\":\"Click\",\"parents\":[3]}]",
    );
    let err = parse(&src).unwrap_err();
    assert_eq!(
        err.to_string(),
        "node 1 (Click) has parent 3 which is not less than its own index 1; edges must point backward"
    );
}

#[test]
fn unknown_kind() {
    let src = ok_wrap("[{\"i\":0,\"kind\":\"Explode\",\"parents\":[]}]");
    let err = parse(&src).unwrap_err();
    assert!(err.to_string().starts_with("node 0 has unknown kind \"Explode\"; expected one of LoadJson"));
}

#[test]
fn foreach_body_out_of_bounds() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"LoadJson\",\"parents\":[],\"name\":\"submission\"},\
          {\"i\":1,\"kind\":\"ForEach\",\"parents\":[0],\"over\":\"answers\",\"body\":99}]",
    );
    let err = parse(&src).unwrap_err();
    assert_eq!(
        err.to_string(),
        "ForEach at index 1 references body index 99 but the arena holds 2 nodes"
    );
}

#[test]
fn valid_minimal_graph_parses_and_validates() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"LoadJson\",\"parents\":[],\"name\":\"submission\"},\
          {\"i\":1,\"kind\":\"Publish\",\"parents\":[0]}]",
    );
    let graph = parse(&src).expect("should parse");
    assert_eq!(graph.nodes.len(), 2);
    validate(&graph).expect("should validate");
}

#[test]
fn locate_type_and_field_are_read_correctly() {
    let src = ok_wrap(
        "[{\"i\":0,\"kind\":\"LoadJson\",\"parents\":[],\"name\":\"submission\"},\
          {\"i\":1,\"kind\":\"Screenshot\",\"parents\":[]},\
          {\"i\":2,\"kind\":\"Scale\",\"parents\":[1],\"width\":1280},\
          {\"i\":3,\"kind\":\"Locate\",\"parents\":[2],\"description\":\"the input\",\
            \"model\":{\"vendor\":\"anthropic\",\"model\":\"opus-performance\",\"think\":\"medium\",\"contextWindow\":1000000}},\
          {\"i\":4,\"kind\":\"Click\",\"parents\":[3]},\
          {\"i\":5,\"kind\":\"Type\",\"parents\":[4,0],\"valueKind\":\"field\",\"value\":\"caseNumber\"},\
          {\"i\":6,\"kind\":\"Publish\",\"parents\":[5]}]",
    );
    let graph = parse(&src).expect("should parse");
    validate(&graph).expect("should validate");

    let locate = &graph.nodes[3];
    match &locate.kind {
        threadbox_ir::NodeKind::Locate { description, model } => {
            assert_eq!(description, "the input");
            let model = model.as_ref().expect("model present");
            assert_eq!(model.vendor.as_deref(), Some("anthropic"));
            assert_eq!(model.model.as_deref(), Some("opus-performance"));
            assert_eq!(model.think.as_deref(), Some("medium"));
            assert_eq!(model.context_window, Some(1000000));
            assert_eq!(model.tier, None);
            assert_eq!(model.role, None);
        }
        other => panic!("expected Locate, got {other:?}"),
    }

    let type_node = &graph.nodes[5];
    match &type_node.kind {
        threadbox_ir::NodeKind::Type { value_kind, value } => {
            assert_eq!(*value_kind, threadbox_ir::ValueKind::Field);
            assert_eq!(value, "caseNumber");
        }
        other => panic!("expected Type, got {other:?}"),
    }
    assert_eq!(type_node.parents, vec![4, 0]);
}
