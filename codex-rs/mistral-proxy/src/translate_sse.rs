//! Translate Mistral Chat Completions streaming SSE into OAI Responses
//! streaming SSE.
//!
//! Implements `Read` so it can be passed directly as a `tiny_http::Response`
//! body. Internally drives a synchronous state machine that consumes upstream
//! SSE lines from a `reqwest::blocking::Response` and emits translated OAI
//! events.
//!
//! ## Event mapping
//!
//! ```text
//! Mistral Chat SSE                    OAI Responses
//! ─────────────────────────────────   ────────────────────────────────────────────
//! first chunk (role:assistant)         response.created + response.in_progress
//!                                      + response.output_item.added (message)
//!                                      + response.content_part.added
//! delta.content (non-empty)            response.output_text.delta
//! delta.tool_calls[n] (new index)      response.output_item.added (function_call)
//! delta.tool_calls[n].function.args    response.function_call_arguments.delta
//! finish_reason=="stop"               response.output_text.done +
//!                                      response.content_part.done +
//!                                      response.output_item.done
//! finish_reason=="tool_calls"          response.function_call_arguments.done +
//!                                      response.output_item.done
//! usage chunk                         (captured for response.completed)
//! [DONE]                               response.completed
//! ```

use std::collections::HashMap;
use std::io;
use std::io::Read;
use std::time;

use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

/// Per-tool-call state during translation.
#[derive(Clone)]
struct ToolCallState {
    /// Stable unique ID used in OAI events (Mistral's tool call id).
    oai_id: String,
    /// Tool name (filled when the function name first arrives).
    tool_name: String,
    /// Accumulated JSON arguments.
    arguments: String,
    /// Whether `response.output_item.added` has been emitted for this tool call.
    announced: bool,
}

/// A `Read` implementation that translates Mistral Chat SSE into OAI Responses
/// SSE on the fly.
pub(crate) struct MistralToOaiStream {
    model: String,
    resp_id: String,
    created_at: u64,
    seq: u64,
    /// Message output item id (the assistant text message).
    msg_id: Option<String>,
    /// Whether we've emitted `response.created` / `response.in_progress`.
    opened: bool,
    /// Whether the text message item has been added.
    msg_item_added: bool,
    /// Accumulated text for the assistant message.
    text_acc: String,
    /// Tool calls by Mistral's streaming index.
    tool_calls: HashMap<usize, ToolCallState>,
    /// Final usage captured from the terminal chunk (if any).
    usage: Option<Value>,
    /// Finish reason from the terminal choice chunk (e.g. "stop", "length",
    /// "tool_calls"). Used to emit `status: "incomplete"` with
    /// `incomplete_details.reason: "max_output_tokens"` when Mistral truncates
    /// at `max_tokens`.
    finish_reason: Option<String>,
    /// Whether `response.completed` has been emitted.
    completed: bool,
    /// Buffered translated bytes not yet consumed by `read()`.
    buf: Vec<u8>,
    /// Upstream response (used as a line source).
    upstream: reqwest::blocking::Response,
    /// Whether the upstream stream is exhausted.
    done: bool,
    /// Residual bytes from the last upstream read (partial SSE line).
    line_buf: Vec<u8>,
    /// Verbose-mode request number; when set, the upstream-reported model from
    /// the first chunk is logged once as proof of which model actually served.
    verbose_req: Option<u64>,
    /// Whether the upstream model has already been logged for this stream.
    upstream_model_logged: bool,
}

impl MistralToOaiStream {
    pub(crate) fn new(
        model: String,
        upstream: reqwest::blocking::Response,
        verbose_req: Option<u64>,
    ) -> Self {
        let resp_id = format!("resp_{}", Uuid::new_v4().simple());
        let created_at = time::SystemTime::now()
            .duration_since(time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self {
            model,
            resp_id,
            created_at,
            seq: 0,
            msg_id: None,
            opened: false,
            msg_item_added: false,
            text_acc: String::new(),
            tool_calls: HashMap::new(),
            usage: None,
            finish_reason: None,
            completed: false,
            buf: Vec::new(),
            upstream,
            done: false,
            line_buf: Vec::new(),
            verbose_req,
            upstream_model_logged: false,
        }
    }
}

impl Read for MistralToOaiStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        loop {
            if !self.buf.is_empty() {
                let n = self.buf.len().min(out.len());
                out[..n].copy_from_slice(&self.buf[..n]);
                self.buf.drain(..n);
                return Ok(n);
            }

            if self.done {
                return Ok(0);
            }

            match self.next_line() {
                Ok(Some(line)) => self.process_line(&line),
                Ok(None) => {
                    self.done = true;
                    return Ok(0);
                }
                Err(e) => return Err(io::Error::other(e.to_string())),
            }
        }
    }
}

impl MistralToOaiStream {
    /// Read the next newline-terminated line from the upstream response.
    fn next_line(&mut self) -> anyhow::Result<Option<String>> {
        let mut tmp = [0u8; 4096];
        loop {
            if let Some(pos) = self.line_buf.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = self.line_buf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line_bytes).trim_end().to_string();
                return Ok(Some(line));
            }
            let n = self.upstream.read(&mut tmp)?;
            if n == 0 {
                if !self.line_buf.is_empty() {
                    let line = String::from_utf8_lossy(&self.line_buf)
                        .trim_end()
                        .to_string();
                    self.line_buf.clear();
                    return Ok(Some(line));
                }
                return Ok(None);
            }
            self.line_buf.extend_from_slice(&tmp[..n]);
        }
    }

    /// Parse one SSE line and push translated OAI events into `self.buf`.
    fn process_line(&mut self, line: &str) {
        if !line.starts_with("data:") {
            return;
        }
        let payload = line["data:".len()..].trim();
        if payload.is_empty() {
            return;
        }
        if payload == "[DONE]" {
            self.emit_completed();
            return;
        }
        let ev: Value = match serde_json::from_str(payload) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("proxy-mistral-ai: non-JSON SSE: {e} — {payload}");
                return;
            }
        };

        if let (Some(req_id), false) = (self.verbose_req, self.upstream_model_logged)
            && let Some(upstream_model) = ev["model"].as_str()
        {
            eprintln!("proxy-mistral-ai: req#{req_id} upstream_model={upstream_model}");
            self.upstream_model_logged = true;
        }

        // Some providers send a final usage-only chunk before [DONE].
        if let Some(usage) = ev.get("usage").filter(|u| !u.is_null()) {
            self.usage = Some(usage.clone());
        }

        let Some(choices) = ev["choices"].as_array() else {
            return;
        };
        let Some(choice) = choices.first() else {
            return;
        };

        let delta = &choice["delta"];

        // Open the response on the first processed choice chunk. Mistral does
        // not guarantee the `role` field appears in the first delta (some
        // streams start with a tool-call delta), so we open unconditionally
        // rather than waiting for `role == "assistant"`.
        if !self.opened {
            self.open_response();
        }

        // Handle text deltas.
        if let Some(text) = delta["content"].as_str()
            && !text.is_empty()
        {
            self.ensure_msg_item();
            self.text_acc.push_str(text);
            self.emit_with_seq(
                "response.output_text.delta",
                json!({
                    "type": "response.output_text.delta",
                    "output_index": 0,
                    "content_index": 0,
                    "item_id": self.msg_id.clone().unwrap_or_default(),
                    "delta": text,
                }),
            );
        }

        // Handle tool call deltas.
        if let Some(tool_calls) = delta["tool_calls"].as_array() {
            for tc in tool_calls {
                let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                self.handle_tool_call_delta(idx, tc);
            }
        }

        // Handle finish reason.
        if let Some(reason) = choice["finish_reason"].as_str()
            && !reason.is_empty()
        {
            self.handle_finish(reason);
        }
    }

    fn open_response(&mut self) {
        self.opened = true;
        let skeleton = self.skeleton("in_progress");
        self.emit("response.created", skeleton.clone());
        self.emit("response.in_progress", skeleton);
    }

    /// Offset for tool-call `output_index` values: when the assistant text
    /// message occupies index 0, tool calls start at index 1; when there is no
    /// text message (only tool calls), they start at index 0.
    fn msg_offset(&self) -> usize {
        if self.msg_id.is_some() { 1 } else { 0 }
    }

    fn ensure_msg_item(&mut self) {
        if self.msg_item_added {
            return;
        }
        self.msg_item_added = true;
        let msg_id = format!("msg_{}", Uuid::new_v4().simple());
        self.msg_id = Some(msg_id.clone());
        self.emit_with_seq(
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "id": msg_id,
                    "type": "message",
                    "status": "in_progress",
                    "role": "assistant",
                    "content": [],
                },
            }),
        );
        self.emit_with_seq(
            "response.content_part.added",
            json!({
                "type": "response.content_part.added",
                "output_index": 0,
                "content_index": 0,
                "item_id": msg_id,
                "part": {"type": "output_text", "text": "", "annotations": []},
            }),
        );
    }

    fn handle_tool_call_delta(&mut self, idx: usize, tc: &Value) {
        // Extract all needed values from `tc` first to avoid holding a mutable
        // borrow of `self.tool_calls` across `emit_with_seq` calls.
        let incoming_id = tc["id"].as_str().map(str::to_string);
        let incoming_name = tc["function"]["name"].as_str().map(str::to_string);
        let incoming_args = tc["function"]["arguments"].as_str().map(str::to_string);
        let has_function_name_field = tc.get("function").is_some_and(|f| f.get("name").is_some());

        // Compute the output index offset dynamically: when there is no text
        // message item, tool calls start at index 0 instead of index 1.
        let output_index = idx + self.msg_offset();

        // Ensure an entry exists; capture the current state snapshot.
        let oai_id = self
            .tool_calls
            .get(&idx)
            .map(|s| s.oai_id.clone())
            .or_else(|| incoming_id.clone())
            .unwrap_or_else(|| format!("call_{}", Uuid::new_v4().simple()));

        let needs_insert = !self.tool_calls.contains_key(&idx);
        if needs_insert {
            self.tool_calls.insert(
                idx,
                ToolCallState {
                    oai_id,
                    tool_name: String::new(),
                    arguments: String::new(),
                    announced: false,
                },
            );
        }

        let state = self
            .tool_calls
            .get_mut(&idx)
            .unwrap_or_else(|| unreachable!());
        if let Some(id) = &incoming_id
            && !id.is_empty()
            && state.oai_id.is_empty()
        {
            state.oai_id = id.clone();
        }
        let updated_oai_id = state.oai_id.clone();
        if let Some(name) = &incoming_name
            && !name.is_empty()
        {
            state.tool_name = name.clone();
        }
        let updated_tool_name = state.tool_name.clone();

        // Decide whether to announce the function_call item.
        let should_announce = !state.announced
            && (!updated_oai_id.is_empty()
                && (!updated_tool_name.is_empty() || has_function_name_field));

        if should_announce {
            state.announced = true;
        }

        // Accumulate argument chunk.
        let arg_chunk = incoming_args.filter(|c| !c.is_empty());
        if let Some(chunk) = &arg_chunk {
            state.arguments.push_str(chunk);
        }
        let updated_args = state.arguments.clone();

        // Now emit — no active mutable borrow of tool_calls.
        if should_announce {
            self.emit_with_seq(
                "response.output_item.added",
                json!({
                    "type": "response.output_item.added",
                    "output_index": output_index,
                    "item": {
                        "id": updated_oai_id,
                        "type": "function_call",
                        "status": "in_progress",
                        "name": updated_tool_name,
                        "arguments": "",
                        "call_id": updated_oai_id,
                    },
                }),
            );
        }

        if let Some(chunk) = &arg_chunk {
            self.emit_with_seq(
                "response.function_call_arguments.delta",
                json!({
                    "type": "response.function_call_arguments.delta",
                    "output_index": output_index,
                    "item_id": updated_oai_id,
                    "delta": chunk,
                }),
            );
            let _ = updated_args; // already accumulated
        }
    }

    fn handle_finish(&mut self, reason: &str) {
        // Close text message if present.
        if self.msg_item_added {
            let msg_id = self.msg_id.clone().unwrap_or_default();
            self.emit_with_seq(
                "response.output_text.done",
                json!({
                    "type": "response.output_text.done",
                    "output_index": 0,
                    "content_index": 0,
                    "item_id": msg_id,
                    "text": self.text_acc.clone(),
                }),
            );
            self.emit_with_seq(
                "response.content_part.done",
                json!({
                    "type": "response.content_part.done",
                    "output_index": 0,
                    "content_index": 0,
                    "item_id": msg_id,
                    "part": {"type": "output_text", "text": self.text_acc.clone(), "annotations": []},
                }),
            );
            self.emit_with_seq(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": 0,
                    "item": {
                        "id": msg_id,
                        "type": "message",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{"type": "output_text", "text": self.text_acc.clone(), "annotations": []}],
                    },
                }),
            );
            self.msg_item_added = false;
        }

        // Close tool calls. Collect states first to avoid borrowing `self`
        // mutably while emitting events.
        let msg_offset = self.msg_offset();
        let tool_calls: Vec<(usize, ToolCallState)> = self
            .tool_calls
            .iter()
            .map(|(idx, state)| (*idx, state.clone()))
            .collect();
        for (idx, state) in tool_calls {
            let output_index = idx + msg_offset;
            if !state.arguments.is_empty() {
                self.emit_with_seq(
                    "response.function_call_arguments.done",
                    json!({
                        "type": "response.function_call_arguments.done",
                        "output_index": output_index,
                        "item_id": state.oai_id,
                        "arguments": state.arguments,
                    }),
                );
            }
            self.emit_with_seq(
                "response.output_item.done",
                json!({
                    "type": "response.output_item.done",
                    "output_index": output_index,
                    "item": {
                        "id": state.oai_id,
                        "type": "function_call",
                        "status": "completed",
                        "name": state.tool_name,
                        "arguments": state.arguments,
                        "call_id": state.oai_id,
                    },
                }),
            );
        }

        // Capture the finish reason for `emit_completed` (called on [DONE]).
        // `length` maps to `status: "incomplete"` with
        // `incomplete_details.reason: "max_output_tokens"`.
        self.finish_reason = Some(reason.to_string());
    }

    fn emit_completed(&mut self) {
        if self.completed {
            return;
        }
        self.completed = true;

        let (input_tokens, output_tokens) = self
            .usage
            .as_ref()
            .map(|u| {
                (
                    u["prompt_tokens"].as_u64().unwrap_or(0),
                    u["completion_tokens"].as_u64().unwrap_or(0),
                )
            })
            .unwrap_or((0, 0));

        // Determine the terminal status from the finish reason. Mistral's
        // `length` means the output was truncated at `max_tokens`; the OAI
        // Responses API represents this as `status: "incomplete"` with
        // `incomplete_details.reason: "max_output_tokens"`.
        let is_length = self.finish_reason.as_deref().is_some_and(|r| r == "length");
        let status = if is_length { "incomplete" } else { "completed" };
        let incomplete_details: Option<Value> = if is_length {
            Some(json!({"reason": "max_output_tokens"}))
        } else {
            None
        };

        // Build output from message + tool calls.
        let text_acc = self.text_acc.clone();
        let mut output: Vec<Value> = Vec::new();
        if let Some(msg_id) = &self.msg_id {
            output.push(json!({
                "id": msg_id,
                "type": "message",
                "status": "completed",
                "role": "assistant",
                "content": [{"type": "output_text", "text": text_acc, "annotations": []}],
            }));
        }
        let mut indices: Vec<usize> = self.tool_calls.keys().copied().collect();
        indices.sort_unstable();
        for idx in indices {
            let state = self.tool_calls.get(&idx).unwrap_or_else(|| unreachable!());
            output.push(json!({
                "id": state.oai_id,
                "type": "function_call",
                "status": "completed",
                "name": state.tool_name,
                "arguments": state.arguments,
                "call_id": state.oai_id,
            }));
        }

        let completed = json!({
            "type": "response.completed",
            "response": {
                "id": self.resp_id,
                "object": "response",
                "created_at": self.created_at,
                "status": status,
                "model": self.model,
                "output": output,
                "usage": {
                    "input_tokens": input_tokens,
                    "input_tokens_details": {"cached_tokens": 0},
                    "output_tokens": output_tokens,
                    "output_tokens_details": {"reasoning_tokens": 0},
                    "total_tokens": input_tokens + output_tokens,
                },
                "incomplete_details": incomplete_details,
                "error": null,
            },
        });
        self.emit("response.completed", completed);
    }

    // ── helpers ───────────────────────────────────────────────────────────────

    fn skeleton(&self, status: &str) -> Value {
        json!({
            "id": self.resp_id,
            "object": "response",
            "created_at": self.created_at,
            "status": status,
            "model": self.model,
            "output": [],
            "usage": null,
            "error": null,
        })
    }

    fn emit(&mut self, event_type: &str, data: Value) {
        let line = format!("event: {event_type}\ndata: {data}\n\n");
        self.buf.extend(line.as_bytes());
    }

    fn emit_with_seq(&mut self, event_type: &str, mut data: Value) {
        let seq = self.seq;
        self.seq += 1;
        if let Value::Object(ref mut map) = data {
            map.insert("sequence_number".to_string(), json!(seq));
        }
        self.emit(event_type, data);
    }
}
