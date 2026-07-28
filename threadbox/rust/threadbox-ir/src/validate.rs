/// The four validators `IR.md` specifies, run in that exact order, first
/// failure wins. See `IR.md` — Validators — for the table this module
/// implements verbatim, including its exact failure-message shapes.
use crate::error::{fail_msg, ParseError};
use crate::{Graph, NodeKind};
use std::collections::HashSet;

pub fn validate_graph(graph: &Graph) -> Result<(), ParseError> {
    let publish_index = validate_single_publish(graph)?;
    validate_no_orphans(graph, publish_index)?;
    validate_retry_bounds(graph)?;
    validate_foreach_bodies(graph)?;
    Ok(())
}

/// Validator 1: exactly one `Publish`. Returns its index so validator 2
/// can use it as the walk root without re-deriving it.
fn validate_single_publish(graph: &Graph) -> Result<usize, ParseError> {
    let publish_indices: Vec<usize> = graph
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::Publish))
        .map(|n| n.index)
        .collect();

    match publish_indices.as_slice() {
        [only] => Ok(*only),
        [] => Err(fail_msg("graph has 0 Publish nodes; exactly one is required".to_string())),
        many => {
            let list = many.iter().map(usize::to_string).collect::<Vec<_>>().join(", ");
            Err(fail_msg(format!(
                "graph has {} Publish nodes at indices [{list}]; exactly one is required",
                many.len()
            )))
        }
    }
}

/// Validator 2: every node is reachable from the `Publish` at
/// `publish_index` by following `parents` and `ForEach.body`. Runs only
/// after validator 1 passes, so `publish_index` names a unique node.
fn validate_no_orphans(graph: &Graph, publish_index: usize) -> Result<(), ParseError> {
    let mut visited: HashSet<usize> = HashSet::new();
    let mut stack: Vec<usize> = vec![publish_index];
    while let Some(i) = stack.pop() {
        if !visited.insert(i) {
            continue;
        }
        let node = &graph.nodes[i];
        for &p in &node.parents {
            stack.push(p);
        }
        if let NodeKind::ForEach { body, .. } = &node.kind {
            stack.push(*body);
        }
    }

    for node in &graph.nodes {
        if !visited.contains(&node.index) {
            return Err(fail_msg(format!(
                "node {} ({}) is unreachable from the Publish at index {publish_index}",
                node.index,
                node.kind.name()
            )));
        }
    }
    Ok(())
}

/// Validator 3: every `Retry` bound is a positive integer.
fn validate_retry_bounds(graph: &Graph) -> Result<(), ParseError> {
    for node in &graph.nodes {
        if let NodeKind::Retry { bound } = &node.kind {
            if *bound <= 0 {
                return Err(fail_msg(format!(
                    "Retry at index {} has bound {bound}; bound must be a positive integer literal",
                    node.index
                )));
            }
        }
    }
    Ok(())
}

/// Validator 4: every `ForEach` names a body below its own index. The
/// reader (`parse.rs`) already guaranteed `body` exists in the arena;
/// this validator owns the semantic "built before" requirement.
fn validate_foreach_bodies(graph: &Graph) -> Result<(), ParseError> {
    for node in &graph.nodes {
        if let NodeKind::ForEach { body, .. } = &node.kind {
            if *body >= node.index {
                return Err(fail_msg(format!(
                    "ForEach at index {} names body index {body}, which is not below it; a body is built before the ForEach that names it",
                    node.index
                )));
            }
        }
    }
    Ok(())
}
