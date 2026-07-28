/// Turns a `threadbox.ir.v1` JSON envelope into a `Graph`, enforcing the
/// structural invariants `IR.md` lists before any validator runs. See
/// `IR.md` — Structural invariants the reader enforces.
use crate::error::{fail, fail_msg, ParseError};
use crate::json::{self, JsonValue};
use crate::{ModelSpec, Node, NodeKind, ValueKind};

pub fn parse_graph(source: &str) -> Result<crate::Graph, ParseError> {
    let root = json::parse(source)?;

    let ir_tag = root
        .get("ir")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| fail_msg("envelope is missing required field \"ir\"".to_string()))?;
    if ir_tag != "threadbox.ir.v1" {
        return Err(fail("ir field", format!("\"{ir_tag}\""), "\"threadbox.ir.v1\""));
    }

    let nodes_json = root
        .get("nodes")
        .and_then(JsonValue::as_arr)
        .ok_or_else(|| fail_msg("envelope is missing required field \"nodes\"".to_string()))?;

    let total_nodes = nodes_json.len();
    let mut nodes: Vec<Node> = Vec::with_capacity(total_nodes);
    for (position, node_json) in nodes_json.iter().enumerate() {
        let node = parse_node(node_json, position, total_nodes)?;
        nodes.push(node);
    }

    Ok(crate::Graph { nodes })
}

fn parse_node(node_json: &JsonValue, position: usize, total_nodes: usize) -> Result<Node, ParseError> {
    let index = node_json
        .get("i")
        .and_then(JsonValue::as_int)
        .ok_or_else(|| fail_msg(format!("node at position {position} is missing required field \"i\"")))?;
    if index != position as i64 {
        return Err(fail_msg(format!(
            "node at position {position} has index {index}; expected {position}"
        )));
    }
    let index = index as usize;

    let kind_name = node_json
        .get("kind")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| fail_msg(format!("node {index} is missing required field \"kind\"")))?;

    let parents_json = node_json
        .get("parents")
        .and_then(JsonValue::as_arr)
        .ok_or_else(|| fail_msg(format!("node {index} is missing required field \"parents\"")))?;

    let mut parents = Vec::with_capacity(parents_json.len());
    for p in parents_json {
        let p = p.as_int().ok_or_else(|| {
            fail_msg(format!("node {index} ({kind_name}) has a non-integer entry in \"parents\""))
        })?;
        let p_usize = check_backward_index(index, kind_name, p, "parent")?;
        parents.push(p_usize);
    }

    let kind = parse_kind(node_json, index, kind_name, total_nodes)?;

    Ok(Node { index, kind, parents })
}

/// Check one index that must point backward within an arena of the
/// referencing node's own size (i.e. `< own_index`), and report which
/// kind of index (`"parent"` or `"body"`) failed, per `IR.md`'s
/// structural-invariant wording. Bounds and backward-ness are checked
/// together here because at parse time the arena has not grown past
/// `own_index` yet, so "in bounds" and "strictly less than" coincide for
/// `parents`. `body` uses a separate, bounds-only check — see
/// `check_body_index` — because its "strictly less than" requirement is
/// validator 4's business, not the reader's.
fn check_backward_index(
    own_index: usize,
    kind_name: &str,
    candidate: i64,
    label: &str,
) -> Result<usize, ParseError> {
    if candidate < 0 || candidate as usize >= own_index {
        return Err(fail_msg(format!(
            "node {own_index} ({kind_name}) has {label} {candidate} which is not less than its own index {own_index}; edges must point backward"
        )));
    }
    Ok(candidate as usize)
}

/// `body` is checked only for existence within the whole arena
/// (`< total_nodes`), per `IR.md`: the reader "checks only that the
/// index exists"; the strictly-less-than-own-index requirement belongs
/// to validator 4, not here — so a `body` pointing forward of its
/// `ForEach` but still within the arena passes the reader and is caught
/// later by `validate::validate_foreach_bodies`.
fn check_body_index(own_index: usize, candidate: i64, total_nodes: usize) -> Result<usize, ParseError> {
    if candidate < 0 || candidate as usize >= total_nodes {
        return Err(fail_msg(format!(
            "ForEach at index {own_index} references body index {candidate} but the arena holds {total_nodes} nodes"
        )));
    }
    Ok(candidate as usize)
}

fn parse_kind(node_json: &JsonValue, index: usize, kind_name: &str, total_nodes: usize) -> Result<NodeKind, ParseError> {
    match kind_name {
        "LoadJson" => {
            let name = required_string(node_json, index, kind_name, "name")?;
            Ok(NodeKind::LoadJson { name })
        }
        "Screenshot" => Ok(NodeKind::Screenshot),
        "Scale" => {
            let width = required_int(node_json, index, kind_name, "width")?;
            Ok(NodeKind::Scale { width })
        }
        "Locate" => {
            let description = required_string(node_json, index, kind_name, "description")?;
            let model = parse_model(node_json, index)?;
            Ok(NodeKind::Locate { description, model })
        }
        "Click" => Ok(NodeKind::Click),
        "Type" => {
            let value_kind_str = required_string(node_json, index, kind_name, "valueKind")?;
            let value_kind = match value_kind_str.as_str() {
                "literal" => ValueKind::Literal,
                "field" => ValueKind::Field,
                other => {
                    return Err(fail_msg(format!(
                        "Type at index {index} has valueKind \"{other}\"; expected \"literal\" or \"field\""
                    )))
                }
            };
            let value = required_string(node_json, index, kind_name, "value")?;
            Ok(NodeKind::Type { value_kind, value })
        }
        "Verify" => {
            let assertion = required_string(node_json, index, kind_name, "assertion")?;
            let model = parse_model(node_json, index)?;
            Ok(NodeKind::Verify { assertion, model })
        }
        "Retry" => {
            let bound = required_int(node_json, index, kind_name, "bound")?;
            Ok(NodeKind::Retry { bound })
        }
        "Fallback" => Ok(NodeKind::Fallback),
        "Branch" => Ok(NodeKind::Branch),
        "ForEach" => {
            let over = required_string(node_json, index, kind_name, "over")?;
            let body_raw = required_int(node_json, index, kind_name, "body")?;
            let body = check_body_index(index, body_raw, total_nodes)?;
            Ok(NodeKind::ForEach { over, body })
        }
        "Publish" => Ok(NodeKind::Publish),
        other => Err(fail_msg(format!(
            "node {index} has unknown kind \"{other}\"; expected one of LoadJson, Screenshot, Scale, Locate, Click, Type, Verify, Retry, Fallback, Branch, ForEach, Publish"
        ))),
    }
}

fn required_string(node_json: &JsonValue, index: usize, kind_name: &str, field: &str) -> Result<String, ParseError> {
    node_json
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| fail_msg(format!("{kind_name} at index {index} is missing required field \"{field}\"")))
}

fn required_int(node_json: &JsonValue, index: usize, kind_name: &str, field: &str) -> Result<i64, ParseError> {
    node_json
        .get(field)
        .and_then(JsonValue::as_int)
        .ok_or_else(|| fail_msg(format!("{kind_name} at index {index} is missing required field \"{field}\"")))
}

fn parse_model(node_json: &JsonValue, index: usize) -> Result<Option<ModelSpec>, ParseError> {
    let Some(model_json) = node_json.get("model") else {
        return Ok(None);
    };
    let fields = model_json
        .as_obj()
        .ok_or_else(|| fail_msg(format!("node {index} has a \"model\" field that is not an object")))?;

    let mut spec = ModelSpec {
        role: None,
        tier: None,
        vendor: None,
        model: None,
        think: None,
        context_window: None,
        driver: None,
    };
    for (key, value) in fields {
        match key.as_str() {
            "role" => spec.role = Some(as_model_str(value, index, "role")?),
            "tier" => spec.tier = Some(as_model_str(value, index, "tier")?),
            "vendor" => spec.vendor = Some(as_model_str(value, index, "vendor")?),
            "model" => spec.model = Some(as_model_str(value, index, "model")?),
            "think" => spec.think = Some(as_model_str(value, index, "think")?),
            "contextWindow" => {
                spec.context_window = Some(value.as_int().ok_or_else(|| {
                    fail_msg(format!("node {index} has model.contextWindow that is not an integer"))
                })?)
            }
            "driver" => spec.driver = Some(as_model_str(value, index, "driver")?),
            other => {
                return Err(fail_msg(format!(
                    "node {index} has unknown model slot \"{other}\"; expected one of role, tier, vendor, model, think, contextWindow, driver"
                )))
            }
        }
    }
    Ok(Some(spec))
}

fn as_model_str(value: &JsonValue, index: usize, slot: &str) -> Result<String, ParseError> {
    value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| fail_msg(format!("node {index} has model.{slot} that is not a string")))
}
