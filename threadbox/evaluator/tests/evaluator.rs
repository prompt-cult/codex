//! Evaluator behaviour, exercised natively. These tests drive the same
//! `Evaluator::step` the wasm export wraps, so they are evidence about the
//! browser runtime and not a parallel implementation of it.

use serde_json::{json, Value};
use threadbox_evaluator::{ir, load};

/// Build an IR envelope around a node array written inline.
fn envelope(nodes: Value) -> String {
    json!({
        "ir": "threadbox.ir.v2",
        "graph": { "id": "test", "version": "0.1.0", "nodes": nodes }
    })
    .to_string()
}

fn config(inputs: Value) -> String {
    json!({ "instanceId": "inst_0001", "runId": "run_0001", "inputs": inputs }).to_string()
}

/// Drive a run to completion, answering every request with `answer`.
/// Returns (the requests that were made, the terminal envelope).
fn drive(
    ir: &str,
    cfg: &str,
    mut answer: impl FnMut(&Value) -> Value,
) -> (Vec<Value>, Value) {
    let mut evaluator = load(ir, cfg).expect("graph should load");
    let mut requests = Vec::new();
    let mut response: Option<Value> = None;
    for _ in 0..10_000 {
        let out = evaluator
            .step(response.as_ref())
            .unwrap_or_else(|e| panic!("step failed: {e}"));
        match out.get("kind").and_then(Value::as_str) {
            Some("run.finished") => return (requests, out),
            Some("tool.request") => {
                let reply = answer(&out);
                requests.push(out);
                response = Some(reply);
            }
            other => panic!("unexpected envelope kind {other:?}"),
        }
    }
    panic!("run did not terminate within the step budget");
}

fn ok(request: &Value, body: Value) -> Value {
    json!({
        "v": 1, "kind": "tool.response",
        "id": request.get("id").unwrap().clone(),
        "ok": true, "body": body
    })
}

#[test]
fn smallest_real_graph_input_guard_terminal() {
    let ir = envelope(json!([
        { "i": 0, "id": "load", "kind": "Input", "parents": [],
          "name": "submission", "schemaUri": "agent-dsl://schemas/submission/acme-motor-quotes/0.1.0" },
        { "i": 1, "id": "has-answers", "kind": "Guard", "parents": [0],
          "mode": "assert", "expr": ".parents[0].document.answers | length > 0" },
        { "i": 2, "id": "stop", "kind": "Terminal", "parents": [1],
          "status": "success", "outcome": "awaiting_human_final_submit" }
    ]));
    let cfg = config(json!({ "submission": { "answers": [{ "id": "a" }] } }));

    let (requests, finished) = drive(&ir, &cfg, |_| panic!("no tool call expected"));

    assert!(requests.is_empty(), "a deterministic graph made a tool call");
    assert_eq!(finished["status"], "success");
    assert_eq!(finished["outcome"], "awaiting_human_final_submit");
}

#[test]
fn a_failed_assertion_is_a_typed_failure_not_a_panic() {
    let ir = envelope(json!([
        { "i": 0, "id": "load", "kind": "Input", "parents": [], "name": "submission", "schemaUri": "x" },
        { "i": 1, "id": "must-be-empty", "kind": "Guard", "parents": [0],
          "mode": "assert", "expr": ".parents[0].document.answers | length == 0" },
        { "i": 2, "id": "stop", "kind": "Terminal", "parents": [1], "status": "success", "outcome": "done" }
    ]));
    let cfg = config(json!({ "submission": { "answers": [1, 2] } }));

    let (_, finished) = drive(&ir, &cfg, |_| panic!("no tool call expected"));
    assert_eq!(finished["status"], "failure");
    assert!(
        finished["outcome"].as_str().unwrap().contains("must-be-empty"),
        "the outcome should name the guard: {finished}"
    );
}

#[test]
fn a_validate_guard_uses_the_generated_jtd() {
    let ir = envelope(json!([
        { "i": 0, "id": "load", "kind": "Input", "parents": [], "name": "doc", "schemaUri": "x" },
        { "i": 1, "id": "unwrap", "kind": "Transform", "parents": [0], "expr": ".parents[0].document" },
        { "i": 2, "id": "conforms", "kind": "Guard", "parents": [1],
          "mode": "validate", "schemaUri": "agent-dsl://schemas/provider-output/localized-control/0.1.0" },
        { "i": 3, "id": "stop", "kind": "Terminal", "parents": [2], "status": "success", "outcome": "done" }
    ]));

    let good = config(json!({ "doc": { "x": 120, "y": 340, "confidence": 0.91 } }));
    let (_, finished) = drive(&ir, &good, |_| panic!("no tool call"));
    assert_eq!(finished["status"], "success");

    // A string where a number belongs must fail at the guard, naming the path.
    let bad = config(json!({ "doc": { "x": "left-ish", "y": 340, "confidence": 0.91 } }));
    let (_, finished) = drive(&ir, &bad, |_| panic!("no tool call"));
    assert_eq!(finished["status"], "failure");
    assert!(
        finished["outcome"].as_str().unwrap().contains("/x"),
        "should name the offending path: {finished}"
    );
}

#[test]
fn a_tool_node_round_trips_through_the_host() {
    let ir = envelope(json!([
        { "i": 0, "id": "observe", "kind": "Tool", "parents": [],
          "tool": "page.observe", "input": "{ freshness: \"new\" }" },
        { "i": 1, "id": "on-step-one", "kind": "Guard", "parents": [0],
          "mode": "assert", "expr": ".parents[0].step == 1" },
        { "i": 2, "id": "stop", "kind": "Terminal", "parents": [1], "status": "success", "outcome": "done" }
    ]));

    let (requests, finished) = drive(&ir, &config(json!({})), |req| {
        assert_eq!(req["tool"], "page.observe");
        assert_eq!(req["body"], json!({ "freshness": "new" }));
        ok(req, json!({ "step": 1, "navigationId": "nav_1" }))
    });

    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["nodeId"], "observe");
    assert_eq!(finished["status"], "success");
    assert_eq!(finished["summary"]["toolCalls"], 1);
}

#[test]
fn a_tool_failure_ends_the_run_naming_the_node_and_code() {
    let ir = envelope(json!([
        { "i": 0, "id": "observe", "kind": "Tool", "parents": [], "tool": "page.observe", "input": "." },
        { "i": 1, "id": "stop", "kind": "Terminal", "parents": [0], "status": "success", "outcome": "done" }
    ]));

    let (_, finished) = drive(&ir, &config(json!({})), |req| {
        json!({
            "v": 1, "kind": "tool.response", "id": req["id"].clone(), "ok": false,
            "error": { "code": "STALE_OBSERVATION", "retryable": true, "message": "page navigated" }
        })
    });

    assert_eq!(finished["status"], "failure");
    let outcome = finished["outcome"].as_str().unwrap();
    assert!(outcome.contains("observe"), "{outcome}");
    assert!(outcome.contains("STALE_OBSERVATION"), "{outcome}");
}

#[test]
fn a_loop_runs_its_body_once_per_element_without_overwriting() {
    let ir = envelope(json!([
        { "i": 0, "id": "load", "kind": "Input", "parents": [], "name": "submission", "schemaUri": "x" },
        { "i": 1, "id": "fields", "kind": "Transform", "parents": [0], "expr": ".parents[0].document.answers" },
        { "i": 2, "id": "each", "kind": "Loop", "parents": [1],
          "var": "field", "over": ".parents[0]", "max": 10,
          "body": { "nodes": [
            { "i": 0, "id": "write", "kind": "Tool", "parents": [],
              "tool": "form.apply", "input": "{ handle: .vars.field.id, value: .vars.field.value }" }
          ] } },
        { "i": 3, "id": "count", "kind": "Transform", "parents": [2], "expr": ".parents[0] | length" },
        { "i": 4, "id": "all-done", "kind": "Guard", "parents": [3], "mode": "assert", "expr": ".parents[0] == 3" },
        { "i": 5, "id": "stop", "kind": "Terminal", "parents": [4], "status": "success", "outcome": "done" }
    ]));
    let cfg = config(json!({ "submission": { "answers": [
        { "id": "a", "value": "1" }, { "id": "b", "value": "2" }, { "id": "c", "value": "3" }
    ] } }));

    let (requests, finished) = drive(&ir, &cfg, |req| ok(req, json!({ "applied": true })));

    assert_eq!(finished["status"], "success", "{finished}");
    assert_eq!(requests.len(), 3);
    let handles: Vec<&str> = requests
        .iter()
        .map(|r| r["body"]["handle"].as_str().unwrap())
        .collect();
    assert_eq!(handles, ["a", "b", "c"], "each iteration sees its own element");

    // Every iteration gets a distinct call and node-instance identity, so no
    // iteration can overwrite an earlier one.
    let call_ids: Vec<&str> = requests.iter().map(|r| r["id"].as_str().unwrap()).collect();
    let node_instances: Vec<&str> = requests
        .iter()
        .map(|r| r["nodeInstanceId"].as_str().unwrap())
        .collect();
    assert_eq!(call_ids, ["call_000001", "call_000002", "call_000003"]);
    assert_eq!(node_instances.len(), 3);
    assert!(
        node_instances.windows(2).all(|w| w[0] != w[1]),
        "node instance ids repeat across iterations: {node_instances:?}"
    );
}

#[test]
fn a_loop_over_more_items_than_its_bound_is_refused_before_any_iteration() {
    let ir = envelope(json!([
        { "i": 0, "id": "load", "kind": "Input", "parents": [], "name": "d", "schemaUri": "x" },
        { "i": 1, "id": "items", "kind": "Transform", "parents": [0], "expr": ".parents[0].document.items" },
        { "i": 2, "id": "each", "kind": "Loop", "parents": [1], "var": "it", "over": ".parents[0]", "max": 2,
          "body": { "nodes": [ { "i": 0, "id": "noop", "kind": "Transform", "parents": [], "expr": ".vars.it" } ] } },
        { "i": 3, "id": "stop", "kind": "Terminal", "parents": [2], "status": "success", "outcome": "done" }
    ]));
    let cfg = config(json!({ "d": { "items": [1, 2, 3] } }));

    let mut evaluator = load(&ir, &cfg).expect("loads");
    let e = evaluator.step(None).expect_err("should refuse the loop");
    assert!(e.message.contains("has 3 items but max 2"), "{}", e.message);
}

#[test]
fn memo_reaches_causally_earlier_nodes_not_just_the_parent() {
    let ir = envelope(json!([
        { "i": 0, "id": "observe", "kind": "Tool", "parents": [], "tool": "page.observe", "input": "." },
        { "i": 1, "id": "a", "kind": "Transform", "parents": [0], "expr": "1" },
        { "i": 2, "id": "b", "kind": "Transform", "parents": [1], "expr": "2" },
        { "i": 3, "id": "reach-back", "kind": "Transform", "parents": [2],
          "expr": ".memo[\"observe\"].navigationId" },
        { "i": 4, "id": "same-nav", "kind": "Guard", "parents": [3],
          "mode": "assert", "expr": ".parents[0] == \"nav_7\"" },
        { "i": 5, "id": "stop", "kind": "Terminal", "parents": [4], "status": "success", "outcome": "done" }
    ]));

    let (_, finished) = drive(&ir, &config(json!({})), |req| {
        ok(req, json!({ "navigationId": "nav_7" }))
    });
    assert_eq!(finished["status"], "success", "{finished}");
}

#[test]
fn an_agent_output_is_validated_at_the_edge() {
    let ir = envelope(json!([
        { "i": 0, "id": "locate", "kind": "Agent", "parents": [],
          "binding": "vision.primary", "prompt": "agent-dsl-fs://prompts/locate@1",
          "input": "{ target: \"the postcode box\" }",
          "outputSchemaUri": "agent-dsl://schemas/provider-output/localized-control/0.1.0" },
        { "i": 1, "id": "stop", "kind": "Terminal", "parents": [0], "status": "success", "outcome": "done" }
    ]));

    // A well-formed provider result passes.
    let (requests, finished) = drive(&ir, &config(json!({})), |req| {
        ok(req, json!({ "result": { "x": 120, "y": 340, "confidence": 0.9 } }))
    });
    assert_eq!(requests[0]["tool"], "model.invoke", "a model call is a generic node");
    assert_eq!(requests[0]["body"]["binding"], "vision.primary");
    assert_eq!(finished["status"], "success");

    // A malformed one fails at the edge rather than downstream.
    let (_, finished) = drive(&ir, &config(json!({})), |req| {
        ok(req, json!({ "result": { "x": "somewhere", "y": 340, "confidence": 0.9 } }))
    });
    assert_eq!(finished["status"], "failure");
    assert!(
        finished["outcome"].as_str().unwrap().contains("does not satisfy"),
        "{finished}"
    );
}

#[test]
fn identity_is_deterministic_across_runs() {
    let ir = envelope(json!([
        { "i": 0, "id": "observe", "kind": "Tool", "parents": [], "tool": "page.observe", "input": "." },
        { "i": 1, "id": "gate", "kind": "Human", "parents": [0], "classification": "form-write", "input": ".parents[0]" },
        { "i": 2, "id": "mark", "kind": "Checkpoint", "parents": [1], "label": "done", "input": ".parents[0]" },
        { "i": 3, "id": "stop", "kind": "Terminal", "parents": [2], "status": "success", "outcome": "done" }
    ]));

    let run_once = || {
        let (requests, finished) = drive(&ir, &config(json!({})), |req| ok(req, json!({ "ok": true })));
        (
            requests
                .iter()
                .map(|r| {
                    format!(
                        "{} {} {} {}",
                        r["sequence"],
                        r["id"].as_str().unwrap(),
                        r["nodeInstanceId"].as_str().unwrap(),
                        r["tool"].as_str().unwrap()
                    )
                })
                .collect::<Vec<_>>(),
            finished,
        )
    };
    let (first, first_end) = run_once();
    let (second, second_end) = run_once();
    assert_eq!(first, second, "two identical runs produced different identity");
    assert_eq!(first_end, second_end);

    // Human and Checkpoint are host concerns reached through the same ABI.
    assert!(first.iter().any(|l| l.ends_with("human.gate")));
    assert!(first.iter().any(|l| l.ends_with("checkpoint.commit")));
}

#[test]
fn reserved_node_kinds_are_refused_by_name() {
    let ir = envelope(json!([
        { "i": 0, "id": "fan", "kind": "Parallel", "parents": [] },
        { "i": 1, "id": "stop", "kind": "Terminal", "parents": [0], "status": "success", "outcome": "done" }
    ]));
    let e = ir::parse(&ir).expect_err("Parallel is reserved");
    assert!(e.message.contains("not implemented in threadbox.ir.v2"), "{}", e.message);
}

#[test]
fn a_response_for_the_wrong_call_is_rejected() {
    let ir = envelope(json!([
        { "i": 0, "id": "observe", "kind": "Tool", "parents": [], "tool": "page.observe", "input": "." },
        { "i": 1, "id": "stop", "kind": "Terminal", "parents": [0], "status": "success", "outcome": "done" }
    ]));
    let mut evaluator = load(&ir, &config(json!({}))).expect("loads");
    evaluator.step(None).expect("first request");
    let e = evaluator
        .step(Some(&json!({ "v": 1, "kind": "tool.response", "id": "call_999999", "ok": true, "body": {} })))
        .expect_err("mismatched id must be refused");
    assert!(e.message.contains("call_000001"), "{}", e.message);
}

#[test]
fn a_branch_runs_only_the_taken_arm() {
    // The economic point of a conditional: the expensive arm must not run
    // when the cheap one already resolved the target.
    let ir = envelope(json!([
        { "i": 0, "id": "resolve", "kind": "Tool", "parents": [],
          "tool": "dom.resolve_accessible", "input": "{ accessibleName: \"First name\" }" },
        { "i": 1, "id": "pick", "kind": "Branch", "parents": [0],
          "cond": ".parents[0].result.resolved",
          "whenTrue": { "nodes": [
            { "i": 0, "id": "cheap", "kind": "Transform", "parents": [],
              "expr": ".memo[\"resolve\"].result.handle" }
          ] },
          "whenFalse": { "nodes": [
            { "i": 0, "id": "expensive", "kind": "Tool", "parents": [],
              "tool": "capture.viewport", "input": "." }
          ] } },
        { "i": 2, "id": "got-handle", "kind": "Guard", "parents": [1],
          "mode": "assert", "expr": ".parents[0] == \"ctl00_abc_txt000\"" },
        { "i": 3, "id": "stop", "kind": "Terminal", "parents": [2], "status": "success", "outcome": "done" }
    ]));

    let (requests, finished) = drive(&ir, &config(json!({})), |req| {
        assert_ne!(req["tool"], "capture.viewport", "the untaken arm must not run");
        ok(req, json!({ "result": { "resolved": true, "handle": "ctl00_abc_txt000" } }))
    });

    assert_eq!(finished["status"], "success", "{finished}");
    assert_eq!(requests.len(), 1, "only the cheap arm should have called a tool");
}

#[test]
fn a_branch_arm_may_not_terminate_the_run() {
    let ir = envelope(json!([
        { "i": 0, "id": "pick", "kind": "Branch", "parents": [],
          "cond": "true",
          "whenTrue": { "nodes": [
            { "i": 0, "id": "early", "kind": "Terminal", "parents": [], "status": "success", "outcome": "sneaky" }
          ] },
          "whenFalse": { "nodes": [ { "i": 0, "id": "n", "kind": "Transform", "parents": [], "expr": "1" } ] } },
        { "i": 1, "id": "stop", "kind": "Terminal", "parents": [0], "status": "success", "outcome": "done" }
    ]));
    let graph = ir::parse(&ir).expect("parses");
    let e = ir::validate(&graph).expect_err("a nested Terminal must be refused");
    assert!(e.message.contains("may not terminate the run"), "{}", e.message);
}
