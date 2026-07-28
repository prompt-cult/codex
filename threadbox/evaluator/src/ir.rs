//! The IR: types, reader, and the structural validators `IR.md` specifies.
//!
//! The IR is not validated by a JTD. A JTD can only report that a value failed
//! at a path; a graph authored by a model and read by a human reviewer
//! deserves "Loop at index 5 (each-field) has max 0". `IR.md` is the
//! specification these validators implement.

use crate::query::{self, Expr};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt;

pub const IR_VERSION: &str = "threadbox.ir.v2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrError {
    pub message: String,
}

impl fmt::Display for IrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn err<T>(message: impl Into<String>) -> Result<T, IrError> {
    Err(IrError {
        message: message.into(),
    })
}

#[derive(Debug, Clone)]
pub struct Graph {
    pub id: String,
    pub version: String,
    pub nodes: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct Node {
    pub i: usize,
    pub id: String,
    pub kind: NodeKind,
    pub parents: Vec<usize>,
}

#[derive(Debug, Clone)]
pub enum NodeKind {
    Input {
        name: String,
        schema_uri: String,
    },
    Tool {
        tool: String,
        input: Expr,
        result_schema_uri: Option<String>,
    },
    Agent {
        binding: String,
        prompt: String,
        input: Expr,
        output_schema_uri: String,
    },
    Transform {
        expr: Expr,
    },
    Guard {
        mode: GuardMode,
    },
    Loop {
        var: String,
        over: Expr,
        max: u32,
        body: Vec<Node>,
    },
    /// The only conditional. It chooses between two nested arenas on a
    /// condition over values that already exist in the graph — it never
    /// observes a live value during construction, and neither arm can
    /// terminate the run.
    Branch {
        cond: Expr,
        when_true: Vec<Node>,
        when_false: Vec<Node>,
    },
    Human {
        classification: String,
        input: Expr,
    },
    Checkpoint {
        label: String,
        input: Expr,
    },
    Terminal {
        status: TerminalStatus,
        outcome: String,
    },
}

#[derive(Debug, Clone)]
pub enum GuardMode {
    Assert { expr: Expr },
    Validate { schema_uri: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalStatus {
    Success,
    Exhausted,
    Cancelled,
    Failure,
}

impl TerminalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TerminalStatus::Success => "success",
            TerminalStatus::Exhausted => "exhausted",
            TerminalStatus::Cancelled => "cancelled",
            TerminalStatus::Failure => "failure",
        }
    }
}

impl NodeKind {
    pub fn name(&self) -> &'static str {
        match self {
            NodeKind::Input { .. } => "Input",
            NodeKind::Tool { .. } => "Tool",
            NodeKind::Agent { .. } => "Agent",
            NodeKind::Transform { .. } => "Transform",
            NodeKind::Guard { .. } => "Guard",
            NodeKind::Loop { .. } => "Loop",
            NodeKind::Branch { .. } => "Branch",
            NodeKind::Human { .. } => "Human",
            NodeKind::Checkpoint { .. } => "Checkpoint",
            NodeKind::Terminal { .. } => "Terminal",
        }
    }
}

// ---------------------------------------------------------------- reader

fn field<'a>(obj: &'a Value, key: &str, at: &str) -> Result<&'a Value, IrError> {
    obj.get(key)
        .ok_or_else(|| IrError {
            message: format!("{at} is missing required field \"{key}\""),
        })
}

fn string_field(obj: &Value, key: &str, at: &str) -> Result<String, IrError> {
    match field(obj, key, at)? {
        Value::String(s) => Ok(s.clone()),
        other => err(format!(
            "{at} has \"{key}\" of type {}; expected a string",
            json_type(other)
        )),
    }
}

fn expr_field(obj: &Value, key: &str, at: &str) -> Result<Expr, IrError> {
    let src = string_field(obj, key, at)?;
    query::parse(&src).map_err(|e| IrError {
        message: format!("{at} has an invalid \"{key}\" expression: {e}"),
    })
}

fn json_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Read a `threadbox.ir.v2` document. Structural invariants that make the
/// document readable at all are enforced here; the seven graph properties are
/// `validate`'s business.
pub fn parse(source: &str) -> Result<Graph, IrError> {
    let root: Value = serde_json::from_str(source).map_err(|e| IrError {
        message: format!("IR is not valid JSON: {e}"),
    })?;

    let tag = string_field(&root, "ir", "the IR envelope")?;
    if tag != IR_VERSION {
        return err(format!(
            "IR envelope declares \"{tag}\"; this evaluator reads {IR_VERSION} only"
        ));
    }

    let graph = field(&root, "graph", "the IR envelope")?;
    let id = string_field(graph, "id", "the graph")?;
    let version = string_field(graph, "version", "the graph")?;

    let nodes_json = match field(graph, "nodes", "the graph")? {
        Value::Array(items) => items,
        other => {
            return err(format!(
                "the graph has \"nodes\" of type {}; expected an array",
                json_type(other)
            ))
        }
    };

    let nodes = parse_nodes(nodes_json, "graph")?;
    Ok(Graph { id, version, nodes })
}

fn parse_nodes(items: &[Value], arena: &str) -> Result<Vec<Node>, IrError> {
    let mut nodes = Vec::with_capacity(items.len());
    for (position, item) in items.iter().enumerate() {
        nodes.push(parse_node(item, position, arena)?);
    }
    Ok(nodes)
}

fn parse_node(node: &Value, position: usize, arena: &str) -> Result<Node, IrError> {
    let at = format!("node at position {position} in the {arena} arena");

    let declared = field(node, "i", &at)?.as_u64().ok_or_else(|| IrError {
        message: format!("{at} has a non-integer \"i\""),
    })? as usize;
    if declared != position {
        return err(format!(
            "{at} declares index {declared}; a node's \"i\" must equal its position"
        ));
    }

    let id = string_field(node, "id", &at)?;
    if id.trim().is_empty() {
        return err(format!("{at} has an empty \"id\"; every node needs a stable id"));
    }
    let at = format!("node {position} ({id})");

    let parents = match field(node, "parents", &at)? {
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for p in items {
                let p = p.as_u64().ok_or_else(|| IrError {
                    message: format!("{at} has a non-integer entry in \"parents\""),
                })? as usize;
                out.push(p);
            }
            out
        }
        other => {
            return err(format!(
                "{at} has \"parents\" of type {}; expected an array",
                json_type(other)
            ))
        }
    };

    let kind_name = string_field(node, "kind", &at)?;
    let kind = match kind_name.as_str() {
        "Input" => NodeKind::Input {
            name: string_field(node, "name", &at)?,
            schema_uri: string_field(node, "schemaUri", &at)?,
        },
        "Tool" => NodeKind::Tool {
            tool: string_field(node, "tool", &at)?,
            input: expr_field(node, "input", &at)?,
            result_schema_uri: node
                .get("resultSchemaUri")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        "Agent" => NodeKind::Agent {
            binding: string_field(node, "binding", &at)?,
            prompt: string_field(node, "prompt", &at)?,
            input: expr_field(node, "input", &at)?,
            output_schema_uri: string_field(node, "outputSchemaUri", &at)?,
        },
        "Transform" => NodeKind::Transform {
            expr: expr_field(node, "expr", &at)?,
        },
        "Guard" => {
            let mode = string_field(node, "mode", &at)?;
            match mode.as_str() {
                "assert" => NodeKind::Guard {
                    mode: GuardMode::Assert {
                        expr: expr_field(node, "expr", &at)?,
                    },
                },
                "validate" => NodeKind::Guard {
                    mode: GuardMode::Validate {
                        schema_uri: string_field(node, "schemaUri", &at)?,
                    },
                },
                other => {
                    return err(format!(
                        "{at} has guard mode \"{other}\"; expected \"assert\" or \"validate\""
                    ))
                }
            }
        }
        "Loop" => {
            let max = field(node, "max", &at)?.as_u64().ok_or_else(|| IrError {
                message: format!("{at} has a non-integer \"max\""),
            })?;
            let body_json = match field(node, "body", &at)? {
                Value::Object(map) => match map.get("nodes") {
                    Some(Value::Array(items)) => items.clone(),
                    _ => {
                        return err(format!(
                            "{at} has a \"body\" without a \"nodes\" array; a loop body is a nested arena"
                        ))
                    }
                },
                other => {
                    return err(format!(
                        "{at} has \"body\" of type {}; expected an object holding a nested arena",
                        json_type(other)
                    ))
                }
            };
            NodeKind::Loop {
                var: string_field(node, "var", &at)?,
                over: expr_field(node, "over", &at)?,
                max: max as u32,
                body: parse_nodes(&body_json, &format!("{id} body"))?,
            }
        }
        "Branch" => {
            let arm = |key: &str| -> Result<Vec<Node>, IrError> {
                match field(node, key, &at)? {
                    Value::Object(map) => match map.get("nodes") {
                        Some(Value::Array(items)) => parse_nodes(items, &format!("{id} {key}")),
                        _ => err(format!(
                            "{at} has a \"{key}\" without a \"nodes\" array; a branch arm is a nested arena"
                        )),
                    },
                    other => err(format!(
                        "{at} has \"{key}\" of type {}; expected an object holding a nested arena",
                        json_type(other)
                    )),
                }
            };
            NodeKind::Branch {
                cond: expr_field(node, "cond", &at)?,
                when_true: arm("whenTrue")?,
                when_false: arm("whenFalse")?,
            }
        }
        "Human" => NodeKind::Human {
            classification: string_field(node, "classification", &at)?,
            input: expr_field(node, "input", &at)?,
        },
        "Checkpoint" => NodeKind::Checkpoint {
            label: string_field(node, "label", &at)?,
            input: expr_field(node, "input", &at)?,
        },
        "Terminal" => {
            let status = string_field(node, "status", &at)?;
            let status = match status.as_str() {
                "success" => TerminalStatus::Success,
                "exhausted" => TerminalStatus::Exhausted,
                "cancelled" => TerminalStatus::Cancelled,
                "failure" => TerminalStatus::Failure,
                other => {
                    return err(format!(
                        "{at} has status \"{other}\"; expected success, exhausted, cancelled, or failure"
                    ))
                }
            };
            NodeKind::Terminal {
                status,
                outcome: string_field(node, "outcome", &at)?,
            }
        }
        "Parallel" | "Join" => {
            return err(format!(
                "{at} is a {kind_name} node, which is reserved but not implemented in {IR_VERSION}"
            ))
        }
        other => {
            return err(format!(
                "{at} has unknown kind \"{other}\"; expected one of Input, Tool, Agent, \
                 Transform, Guard, Loop, Branch, Human, Checkpoint, Terminal"
            ))
        }
    };

    Ok(Node {
        i: position,
        id,
        kind,
        parents,
    })
}

// ---------------------------------------------------------------- validators

/// The properties `IR.md` lists, in order. The first failure stops the run,
/// because a graph that fails any of them is not run.
pub fn validate(graph: &Graph) -> Result<(), IrError> {
    validate_arena(&graph.nodes, "graph", true)
}

fn validate_arena(nodes: &[Node], arena: &str, is_top_level: bool) -> Result<(), IrError> {
    // 2 — parents point backward and are in bounds.
    for node in nodes {
        for &p in &node.parents {
            if p >= node.i {
                return err(format!(
                    "node {} ({}) has parent {p} which is not less than its own index {}; \
                     edges must point backward",
                    node.i, node.id, node.i
                ));
            }
        }
    }

    // 4 — ids are unique within the arena.
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for node in nodes {
        if !seen.insert(node.id.as_str()) {
            return err(format!(
                "the {arena} arena has two nodes with id \"{}\"; ids must be unique within an arena",
                node.id
            ));
        }
    }

    // 3 — exactly one Terminal at top level, none nested.
    let terminals: Vec<&Node> = nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Terminal { .. }))
        .collect();
    if is_top_level {
        match terminals.len() {
            1 => {}
            0 => return err("graph has 0 Terminal nodes; exactly one is required".to_string()),
            n => {
                let list = terminals
                    .iter()
                    .map(|t| t.i.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                return err(format!(
                    "graph has {n} Terminal nodes at indices [{list}]; exactly one is required"
                ));
            }
        }
    } else if let Some(t) = terminals.first() {
        return err(format!(
            "node {} ({}) is a Terminal inside the {arena} arena; a loop body may not terminate the run",
            t.i, t.id
        ));
    }

    // 5 — loop bodies are non-empty with a positive bound, and validate recursively.
    for node in nodes {
        if let NodeKind::Loop { max, body, var, .. } = &node.kind {
            if *max == 0 {
                return err(format!(
                    "Loop at index {} ({}) has max 0; max must be a positive integer literal",
                    node.i, node.id
                ));
            }
            if body.is_empty() {
                return err(format!(
                    "Loop at index {} ({}) has an empty body; every loop names a non-empty nested arena",
                    node.i, node.id
                ));
            }
            if var.trim().is_empty() {
                return err(format!(
                    "Loop at index {} ({}) has an empty \"var\"; the loop variable needs a name",
                    node.i, node.id
                ));
            }
            validate_arena(body, &node.id, false)?;
        }
        if let NodeKind::Branch {
            when_true,
            when_false,
            ..
        } = &node.kind
        {
            if when_true.is_empty() || when_false.is_empty() {
                return err(format!(
                    "Branch at index {} ({}) has an empty arm; both arms are non-empty nested arenas",
                    node.i, node.id
                ));
            }
            validate_arena(when_true, &format!("{} whenTrue", node.id), false)?;
            validate_arena(when_false, &format!("{} whenFalse", node.id), false)?;
        }
    }

    // 6 — reachability from the terminal (top level) or the body tail (nested).
    let root = if is_top_level {
        terminals[0].i
    } else {
        nodes.len() - 1
    };
    let mut reachable = vec![false; nodes.len()];
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        if reachable[i] {
            continue;
        }
        reachable[i] = true;
        for &p in &nodes[i].parents {
            stack.push(p);
        }
    }
    for node in nodes {
        if !reachable[node.i] {
            let from = if is_top_level {
                format!("the Terminal at index {root}")
            } else {
                format!("the body tail at index {root}")
            };
            return err(format!(
                "node {} ({}) is unreachable from {from}; every node must contribute to the result",
                node.i, node.kind.name()
            ));
        }
    }

    Ok(())
}
