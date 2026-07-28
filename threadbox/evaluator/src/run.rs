//! The step machine.
//!
//! WebAssembly cannot block on a promise, so the evaluator never calls the
//! host: the host calls it. Each `step` advances until a node needs the world,
//! and returns the request that would satisfy it. See `ABI.md` — Step machine.
//!
//! Everything here is pure. There is no clock, no randomness, no I/O. Identity
//! is minted from counters so that two runs of the same graph over the same
//! inputs produce byte-identical logs.

use crate::generated;
use crate::ir::{Graph, GuardMode, Node, NodeKind, TerminalStatus};
use crate::query;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalError {
    pub message: String,
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

fn err<T>(message: impl Into<String>) -> Result<T, EvalError> {
    Err(EvalError {
        message: message.into(),
    })
}

/// Run configuration, supplied by the host at `tb_start`. Input documents are
/// mounted here rather than fetched, because the evaluator has no filesystem.
#[derive(Debug, Clone, Default)]
pub struct Config {
    pub instance_id: String,
    pub run_id: String,
    pub inputs: BTreeMap<String, Value>,
}

impl Config {
    pub fn from_json(source: &str) -> Result<Config, EvalError> {
        let value: Value = match serde_json::from_str(source) {
            Ok(v) => v,
            Err(e) => return err(format!("run configuration is not valid JSON: {e}")),
        };
        let mut inputs = BTreeMap::new();
        if let Some(Value::Object(map)) = value.get("inputs") {
            for (name, doc) in map {
                inputs.insert(name.clone(), doc.clone());
            }
        }
        Ok(Config {
            instance_id: value
                .get("instanceId")
                .and_then(Value::as_str)
                .unwrap_or("inst_0001")
                .to_string(),
            run_id: value
                .get("runId")
                .and_then(Value::as_str)
                .unwrap_or("run_0001")
                .to_string(),
            inputs,
        })
    }
}

struct LoopState {
    /// Index of the `Loop` node in the *parent* frame.
    parent_node: usize,
    var: String,
    items: Vec<Value>,
    cursor: usize,
    collected: Vec<Value>,
}

/// What happens when a frame runs off its end. The top-level arena never
/// does — validation guarantees it reaches a Terminal first.
enum FrameExit {
    Top,
    Loop(LoopState),
    /// A branch arm: its tail value becomes the `Branch` node's value.
    Branch { parent_node: usize },
}

struct Frame {
    nodes: Vec<Node>,
    next: usize,
    values: Vec<Value>,
    vars: BTreeMap<String, Value>,
    exit: FrameExit,
}

impl Frame {
    fn new(nodes: Vec<Node>, vars: BTreeMap<String, Value>, exit: FrameExit) -> Frame {
        let len = nodes.len();
        Frame {
            nodes,
            next: 0,
            values: vec![Value::Null; len],
            vars,
            exit,
        }
    }

    fn restart(&mut self) {
        self.next = 0;
        for slot in &mut self.values {
            *slot = Value::Null;
        }
    }
}

struct Pending {
    frame: usize,
    node: usize,
    call_id: String,
}

pub struct Evaluator {
    config: Config,
    frames: Vec<Frame>,
    memo: BTreeMap<String, Value>,
    pending: Option<Pending>,
    sequence: u32,
    node_instances: u32,
    calls: u32,
    nodes_executed: u32,
    artifacts: u32,
    finished: bool,
}

impl Evaluator {
    pub fn new(graph: Graph, config: Config) -> Evaluator {
        Evaluator {
            config,
            frames: vec![Frame::new(graph.nodes, BTreeMap::new(), FrameExit::Top)],
            memo: BTreeMap::new(),
            pending: None,
            sequence: 0,
            node_instances: 0,
            calls: 0,
            nodes_executed: 0,
            artifacts: 0,
            finished: false,
        }
    }

    /// Advance. `response` is `None` on the first call, otherwise the
    /// `tool.response` for the outstanding request. Returns either a
    /// `tool.request` or a `run.finished`.
    pub fn step(&mut self, response: Option<&Value>) -> Result<Value, EvalError> {
        if self.finished {
            return err("run has already finished; the host must stop stepping");
        }
        if let Some(response) = response {
            if let Some(finished) = self.accept_response(response)? {
                return Ok(finished);
            }
        } else if self.pending.is_some() {
            return err("a request is outstanding but the host supplied no response");
        }
        self.advance()
    }

    fn accept_response(&mut self, response: &Value) -> Result<Option<Value>, EvalError> {
        let Some(pending) = self.pending.take() else {
            return err("host supplied a response but no request was outstanding");
        };

        let id = response.get("id").and_then(Value::as_str).unwrap_or_default();
        if id != pending.call_id {
            return err(format!(
                "host supplied a response for \"{id}\" but \"{}\" was outstanding",
                pending.call_id
            ));
        }

        let ok = response.get("ok").and_then(Value::as_bool).unwrap_or(false);
        if !ok {
            let error = response.get("error").cloned().unwrap_or(Value::Null);
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("TOOL_FAILED");
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the tool reported a failure with no message");
            let node_id = self.frames[pending.frame].nodes[pending.node].id.clone();
            return Ok(Some(self.finish(
                TerminalStatus::Failure,
                &format!("{node_id} failed: {code}: {message}"),
            )));
        }

        let body = response.get("body").cloned().unwrap_or(Value::Null);

        // An Agent node's output is validated at the edge, so a malformed
        // provider response fails here rather than downstream.
        let node = &self.frames[pending.frame].nodes[pending.node];
        if let NodeKind::Agent {
            output_schema_uri, ..
        } = &node.kind
        {
            let uri = output_schema_uri.clone();
            let node_id = node.id.clone();
            let subject = body.get("result").cloned().unwrap_or_else(|| body.clone());
            if let Err(problems) = generated::validate_against(&uri, &subject) {
                return Ok(Some(self.finish(
                    TerminalStatus::Failure,
                    &format!("{node_id} produced output that does not satisfy {uri}: {}", problems.join("; ")),
                )));
            }
        }

        self.artifacts += body
            .get("files")
            .and_then(|f| f.get("items"))
            .and_then(Value::as_array)
            .map(|items| items.len() as u32)
            .unwrap_or(0);

        self.complete_node(pending.frame, pending.node, body);
        Ok(None)
    }

    fn complete_node(&mut self, frame: usize, node: usize, value: Value) {
        let id = self.frames[frame].nodes[node].id.clone();
        self.frames[frame].values[node] = value.clone();
        self.memo.insert(id, value);
        self.frames[frame].next = node + 1;
        self.nodes_executed += 1;
    }

    fn context(&self, frame: usize, node: usize) -> Value {
        let f = &self.frames[frame];
        let parents: Vec<Value> = f.nodes[node]
            .parents
            .iter()
            .map(|&p| f.values[p].clone())
            .collect();
        let mut vars = Map::new();
        for (k, v) in &f.vars {
            vars.insert(k.clone(), v.clone());
        }
        let mut memo = Map::new();
        for (k, v) in &self.memo {
            memo.insert(k.clone(), v.clone());
        }
        json!({ "parents": parents, "vars": Value::Object(vars), "memo": Value::Object(memo) })
    }

    fn eval(&self, frame: usize, node: usize, expr: &query::Expr) -> Result<Value, EvalError> {
        let ctx = self.context(frame, node);
        query::eval(expr, &ctx).map_err(|e| EvalError {
            message: format!(
                "node {} ({}) failed to evaluate an expression: {e}",
                self.frames[frame].nodes[node].i,
                self.frames[frame].nodes[node].id
            ),
        })
    }

    fn advance(&mut self) -> Result<Value, EvalError> {
        loop {
            let frame_index = self.frames.len() - 1;
            let next = self.frames[frame_index].next;

            if next >= self.frames[frame_index].nodes.len() {
                if let Some(finished) = self.close_frame(frame_index)? {
                    return Ok(finished);
                }
                continue;
            }

            if let Some(output) = self.execute(frame_index, next)? {
                return Ok(output);
            }
        }
    }

    /// A frame ran off its end. For a loop body that means one iteration
    /// finished; for the top-level arena it means the graph had no Terminal,
    /// which validation already rejected.
    fn close_frame(&mut self, frame_index: usize) -> Result<Option<Value>, EvalError> {
        let tail = self.frames[frame_index]
            .values
            .last()
            .cloned()
            .unwrap_or(Value::Null);

        match &mut self.frames[frame_index].exit {
            FrameExit::Top => {
                err("the top-level arena ran to its end without reaching a Terminal")
            }

            FrameExit::Branch { parent_node } => {
                let parent_node = *parent_node;
                self.frames.pop();
                let parent = self.frames.len() - 1;
                self.complete_node(parent, parent_node, tail);
                Ok(None)
            }

            FrameExit::Loop(state) => {
                state.collected.push(tail);
                state.cursor += 1;

                if state.cursor < state.items.len() {
                    let var = state.var.clone();
                    let item = state.items[state.cursor].clone();
                    self.frames[frame_index].vars.insert(var, item);
                    self.frames[frame_index].restart();
                    self.node_instances += 1;
                    return Ok(None);
                }

                let frame = self.frames.pop().expect("frame exists");
                let FrameExit::Loop(state) = frame.exit else {
                    unreachable!("matched as a loop above")
                };
                let value = Value::Array(state.collected);
                let parent = self.frames.len() - 1;
                self.complete_node(parent, state.parent_node, value);
                Ok(None)
            }
        }
    }

    /// Execute one node. Returns `Some(envelope)` when the host must act.
    fn execute(&mut self, frame: usize, node: usize) -> Result<Option<Value>, EvalError> {
        self.node_instances += 1;
        let kind = self.frames[frame].nodes[node].kind.clone();

        match kind {
            NodeKind::Input { name, .. } => {
                let Some(document) = self.config.inputs.get(&name).cloned() else {
                    let available: Vec<&str> = self.config.inputs.keys().map(String::as_str).collect();
                    return err(format!(
                        "input \"{name}\" was not mounted by the host; available inputs: [{}]",
                        available.join(", ")
                    ));
                };
                self.complete_node(frame, node, json!({ "name": name, "document": document }));
                Ok(None)
            }

            NodeKind::Transform { expr } => {
                let value = self.eval(frame, node, &expr)?;
                self.complete_node(frame, node, value);
                Ok(None)
            }

            NodeKind::Guard { mode } => {
                let passthrough = self.frames[frame].nodes[node]
                    .parents
                    .first()
                    .map(|&p| self.frames[frame].values[p].clone())
                    .unwrap_or(Value::Null);
                let node_id = self.frames[frame].nodes[node].id.clone();

                match mode {
                    GuardMode::Assert { expr } => {
                        let outcome = self.eval(frame, node, &expr)?;
                        let held = !matches!(outcome, Value::Null | Value::Bool(false));
                        if !held {
                            return Ok(Some(self.finish(
                                TerminalStatus::Failure,
                                &format!("guard {node_id} did not hold"),
                            )));
                        }
                    }
                    GuardMode::Validate { schema_uri } => {
                        if let Err(problems) = generated::validate_against(&schema_uri, &passthrough) {
                            return Ok(Some(self.finish(
                                TerminalStatus::Failure,
                                &format!(
                                    "guard {node_id} rejected a value that does not satisfy {schema_uri}: {}",
                                    problems.join("; ")
                                ),
                            )));
                        }
                    }
                }
                // A guard is a gate, not a projection: it passes its input on.
                self.complete_node(frame, node, passthrough);
                Ok(None)
            }

            NodeKind::Loop {
                var,
                over,
                max,
                body,
            } => {
                let items = match self.eval(frame, node, &over)? {
                    Value::Array(items) => items,
                    other => {
                        return err(format!(
                            "Loop {} evaluated \"over\" to {}; a loop needs an array",
                            self.frames[frame].nodes[node].id,
                            match other {
                                Value::Null => "null",
                                Value::Bool(_) => "a boolean",
                                Value::Number(_) => "a number",
                                Value::String(_) => "a string",
                                Value::Object(_) => "an object",
                                Value::Array(_) => unreachable!("matched above"),
                            }
                        ))
                    }
                };

                if items.len() as u32 > max {
                    return err(format!(
                        "Loop {} has {} items but max {max}; the bound is exceeded before any iteration runs",
                        self.frames[frame].nodes[node].id,
                        items.len()
                    ));
                }

                if items.is_empty() {
                    self.complete_node(frame, node, Value::Array(Vec::new()));
                    return Ok(None);
                }

                let mut vars = self.frames[frame].vars.clone();
                vars.insert(var.clone(), items[0].clone());
                let state = LoopState {
                    parent_node: node,
                    var,
                    items,
                    cursor: 0,
                    collected: Vec::new(),
                };
                self.frames.push(Frame::new(body, vars, FrameExit::Loop(state)));
                Ok(None)
            }

            NodeKind::Branch {
                cond,
                when_true,
                when_false,
            } => {
                let outcome = self.eval(frame, node, &cond)?;
                let taken = !matches!(outcome, Value::Null | Value::Bool(false));
                let arm = if taken { when_true } else { when_false };
                let vars = self.frames[frame].vars.clone();
                self.frames
                    .push(Frame::new(arm, vars, FrameExit::Branch { parent_node: node }));
                Ok(None)
            }

            NodeKind::Terminal { status, outcome } => Ok(Some(self.finish(status, &outcome))),

            NodeKind::Tool {
                tool,
                input,
                result_schema_uri,
            } => {
                let body = self.eval(frame, node, &input)?;
                Ok(Some(self.request(frame, node, &tool, body, result_schema_uri)))
            }

            NodeKind::Agent {
                binding,
                prompt,
                input,
                output_schema_uri,
            } => {
                let inner = self.eval(frame, node, &input)?;
                let body = json!({
                    "binding": binding,
                    "prompt": prompt,
                    "outputSchemaUri": output_schema_uri,
                    "input": inner,
                });
                Ok(Some(self.request(frame, node, "model.invoke", body, None)))
            }

            NodeKind::Human {
                classification,
                input,
            } => {
                let inner = self.eval(frame, node, &input)?;
                let body = json!({ "classification": classification, "proposal": inner });
                Ok(Some(self.request(frame, node, "human.gate", body, None)))
            }

            NodeKind::Checkpoint { label, input } => {
                let inner = self.eval(frame, node, &input)?;
                let body = json!({ "label": label, "data": inner });
                Ok(Some(self.request(frame, node, "checkpoint.commit", body, None)))
            }
        }
    }

    fn request(
        &mut self,
        frame: usize,
        node: usize,
        tool: &str,
        body: Value,
        result_schema_uri: Option<String>,
    ) -> Value {
        self.calls += 1;
        self.sequence += 1;
        let call_id = format!("call_{:06}", self.calls);
        let node_ref = &self.frames[frame].nodes[node];

        let mut envelope = json!({
            "v": 1,
            "kind": "tool.request",
            "id": call_id,
            "instanceId": self.config.instance_id,
            "runId": self.config.run_id,
            "nodeId": node_ref.id,
            "nodeInstanceId": format!("nodei_{:06}", self.node_instances),
            "attempt": 1,
            "sequence": self.sequence,
            "tool": tool,
            "toolVersion": "0.1.0",
            "body": body,
        });
        if let Some(uri) = result_schema_uri {
            envelope["resultSchemaUri"] = Value::String(uri);
        }

        self.pending = Some(Pending {
            frame,
            node,
            call_id,
        });
        envelope
    }

    fn finish(&mut self, status: TerminalStatus, outcome: &str) -> Value {
        self.finished = true;
        self.sequence += 1;
        json!({
            "v": 1,
            "kind": "run.finished",
            "status": status.as_str(),
            "outcome": outcome,
            "sequence": self.sequence,
            "summary": {
                "nodesExecuted": self.nodes_executed,
                "toolCalls": self.calls,
                "artifacts": self.artifacts,
            }
        })
    }
}
