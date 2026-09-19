//! Translate between OAI Responses API and Chat Chat Completions API
//! request/response formats.
//!
//! Chat's Chat Completions API is OpenAI-compatible: tools pass through
//! unchanged, `input` message items flatten to `messages`, and reasoning
//! fields are dropped (Chat has no Responses-style reasoning effort).
//!
//! ## Field mapping
//!
//! ```text
//! OAI Responses              →  Chat Chat Completions
//! ─────────────────────────────  ────────────────────────────────────
//! instructions               →  messages[0] {role:"system", content:...}
//! input[].type == "message" →  messages[] {role, content}
//! input[].type == "function_call"          → assistant {tool_calls:[...]}
//! input[].type == "function_call_output"   → {role:"tool", tool_call_id, content}
//! tools[]                    →  tools[]  (identical OpenAI function schema)
//! max_output_tokens          →  max_tokens
//! stream                     →  stream
//! reasoning_effort/summary  →  (dropped)
//! ```

use serde_json::Map;
use serde_json::Value;
use serde_json::json;
use uuid::Uuid;

/// Convert an OAI Responses API request body into a Chat Chat Completions body.
pub(crate) fn oai_to_chat(oai: &Value) -> Value {
    let model = oai["model"].as_str().unwrap_or("");
    let is_stream = oai["stream"].as_bool().unwrap_or(false);
    let max_tokens = oai["max_output_tokens"].as_u64().unwrap_or(16384);

    let instructions = oai["instructions"].as_str();
    let input = oai["input"].as_array();

    let (system_from_input, messages) = convert_input_to_messages(input);
    // The OAI Responses API treats top-level `instructions` as a prepended
    // system turn; system/developer messages embedded in `input` are
    // additional context, not alternatives. Concatenate so neither is lost.
    let system = match (instructions, system_from_input.as_deref()) {
        (Some(a), Some(b)) => Some(format!("{a}\n\n{b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, b) => b.map(str::to_string),
    };

    let tools = convert_tools(oai["tools"].as_array());

    // Build the final messages array: optional system message first, then the
    // converted conversation items.
    let mut all_messages: Vec<Value> = Vec::new();
    if let Some(sys) = system {
        all_messages.push(json!({"role": "system", "content": sys}));
    }
    all_messages.extend(messages);

    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": all_messages,
        "stream": is_stream,
    });

    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }

    body
}

/// Convert a non-streaming Chat Chat Completions response into OAI Responses format.
pub(crate) fn chat_response_to_oai(chat: &Value, model: &str) -> Value {
    let resp_id = format!("resp_{}", Uuid::new_v4().simple());
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());

    let mut output = Vec::new();

    // Determine terminal status from the first choice's finish_reason.
    // Chat's `length` means the output was truncated at `max_tokens`; the
    // OAI Responses API represents this as `status: "incomplete"` with
    // `incomplete_details.reason: "max_output_tokens"`.
    let finish_reason = chat["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .and_then(|choice| choice["finish_reason"].as_str())
        .unwrap_or("");
    let is_length = finish_reason == "length";
    let status = if is_length { "incomplete" } else { "completed" };
    let incomplete_details: Option<Value> = if is_length {
        Some(json!({"reason": "max_output_tokens"}))
    } else {
        None
    };

    if let Some(choices) = chat["choices"].as_array() {
        for choice in choices {
            let message = &choice["message"];
            let role = message["role"].as_str().unwrap_or("assistant");

            // Reasoning models return assistant `content` as a list of typed
            // blocks instead of a string, so both shapes must be handled here.
            let mut text_parts: Vec<&str> = Vec::new();
            let mut reasoning_parts: Vec<&str> = Vec::new();
            match &message["content"] {
                Value::String(content) if !content.is_empty() => text_parts.push(content),
                Value::Array(blocks) => {
                    for block in blocks {
                        match block["type"].as_str() {
                            Some("text") => {
                                if let Some(text) = block["text"].as_str()
                                    && !text.is_empty()
                                {
                                    text_parts.push(text);
                                }
                            }
                            Some("thinking") => {
                                if let Some(inner) = block["thinking"].as_array() {
                                    for part in inner {
                                        if part["type"].as_str() == Some("text")
                                            && let Some(text) = part["text"].as_str()
                                            && !text.is_empty()
                                        {
                                            reasoning_parts.push(text);
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }

            if !reasoning_parts.is_empty() {
                let rs_id = format!("rs_{}", Uuid::new_v4().simple());
                output.push(json!({
                    "id": rs_id,
                    "type": "reasoning",
                    "summary": [],
                    "content": [{"type": "reasoning_text", "text": reasoning_parts.concat()}],
                }));
            }

            if !text_parts.is_empty() {
                let msg_id = format!("msg_{}", Uuid::new_v4().simple());
                output.push(json!({
                    "id": msg_id,
                    "type": "message",
                    "status": "completed",
                    "role": role,
                    "content": [
                        {"type": "output_text", "text": text_parts.concat(), "annotations": []}
                    ],
                }));
            }

            // Tool calls → function_call output items.
            if let Some(tool_calls) = message["tool_calls"].as_array() {
                for tc in tool_calls {
                    let call_id = tc["id"].as_str().unwrap_or("").to_string();
                    let name = tc["function"]["name"].as_str().unwrap_or("");
                    let arguments = tc["function"]["arguments"].as_str().unwrap_or("{}");
                    output.push(json!({
                        "id": call_id,
                        "type": "function_call",
                        "status": "completed",
                        "name": name,
                        "call_id": call_id,
                        "arguments": arguments,
                    }));
                }
            }
        }
    }

    let usage = &chat["usage"];
    let input_tokens = usage["prompt_tokens"].as_u64().unwrap_or(0);
    let output_tokens = usage["completion_tokens"].as_u64().unwrap_or(0);

    json!({
        "id": resp_id,
        "object": "response",
        "created_at": created_at,
        "status": status,
        "model": model,
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
    })
}

/// Convert OAI `input` array to (optional system prompt, Chat messages).
fn convert_input_to_messages(input: Option<&Vec<Value>>) -> (Option<String>, Vec<Value>) {
    let Some(items) = input else {
        return (None, vec![]);
    };

    let mut messages: Vec<Value> = Vec::new();
    let mut system: Option<String> = None;

    for item in items {
        let itype = item["type"].as_str().unwrap_or("message");
        let role = item["role"].as_str().unwrap_or("user");

        match itype {
            "message" => {
                let text = extract_message_text(item);

                if role == "system" || role == "developer" {
                    system = Some(match system {
                        Some(existing) => format!("{existing}\n\n{text}"),
                        None => text,
                    });
                } else {
                    messages.push(json!({"role": role, "content": text}));
                }
            }
            "function_call" => {
                let call_id = item["call_id"]
                    .as_str()
                    .or_else(|| item["id"].as_str())
                    .unwrap_or("call_unknown")
                    .to_string();
                let name = item["name"].as_str().unwrap_or("");
                let arguments = item["arguments"].as_str().unwrap_or("{}");

                let tool_call = json!({
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": name,
                        "arguments": arguments,
                    },
                });

                // Append to existing assistant message or create new one.
                if let Some(last) = messages.last_mut()
                    && last["role"].as_str() == Some("assistant")
                {
                    let tool_calls = last
                        .as_object_mut()
                        .unwrap_or_else(|| unreachable!())
                        .entry("tool_calls".to_string())
                        .or_insert(Value::Array(vec![]));
                    if let Some(arr) = tool_calls.as_array_mut() {
                        arr.push(tool_call);
                    }
                    continue;
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [tool_call],
                }));
            }
            "function_call_output" => {
                let call_id = item["call_id"].as_str().unwrap_or("");
                let output = item["output"].as_str().unwrap_or("");
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": call_id,
                    "content": output,
                }));
            }
            _ => {}
        }
    }

    (system, messages)
}

/// Extract text content from an OAI message item.
fn extract_message_text(item: &Value) -> String {
    match &item["content"] {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            let mut texts = Vec::new();
            for part in parts {
                if let Some("input_text" | "text" | "output_text") = part["type"].as_str()
                    && let Some(t) = part["text"].as_str()
                {
                    texts.push(t.to_string());
                }
            }
            texts.join("\n")
        }
        _ => String::new(),
    }
}

/// Convert OAI Responses tool definitions to Chat (OpenAI-compatible) tool format.
fn convert_tools(tools: Option<&Vec<Value>>) -> Vec<Value> {
    let Some(tools) = tools else {
        return vec![];
    };

    let mut result = Vec::new();
    for tool in tools {
        let (name, description, parameters) = if tool["type"].as_str() == Some("function") {
            if let Some(func) = tool.get("function") {
                (
                    func["name"].as_str().unwrap_or(""),
                    func["description"].as_str().unwrap_or(""),
                    func.get("parameters")
                        .cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}})),
                )
            } else {
                (
                    tool["name"].as_str().unwrap_or(""),
                    tool["description"].as_str().unwrap_or(""),
                    tool.get("parameters")
                        .cloned()
                        .unwrap_or(json!({"type": "object", "properties": {}})),
                )
            }
        } else {
            (
                tool["name"].as_str().unwrap_or(""),
                tool["description"].as_str().unwrap_or(""),
                tool.get("parameters")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
            )
        };

        if name.is_empty() {
            continue;
        }

        result.push(json!({
            "type": "function",
            "function": {
                "name": name,
                "description": description,
                "parameters": parameters,
            },
        }));
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn instructions_and_input_system_message_are_concatenated() {
        let oai = json!({
            "model": "m",
            "instructions": "Top-level instructions.",
            "input": [
                {"type": "message", "role": "developer", "content": "Input-embedded context."},
                {"type": "message", "role": "user", "content": "Hello"}
            ]
        });
        let result = oai_to_chat(&oai);
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "Top-level instructions.\n\nInput-embedded context."
        );
        assert_eq!(result["messages"][1]["role"], "user");
    }

    #[test]
    fn input_system_message_survives_without_instructions() {
        let oai = json!({
            "model": "m",
            "input": [
                {"type": "message", "role": "system", "content": "Only embedded."},
                {"type": "message", "role": "user", "content": "Hello"}
            ]
        });
        let result = oai_to_chat(&oai);
        assert_eq!(result["messages"][0]["content"], "Only embedded.");
    }

    #[test]
    fn test_basic_request_translation() {
        let oai = json!({
            "model": "zai-glm-5-2",
            "stream": true,
            "instructions": "You are helpful.",
            "input": [
                {"type": "message", "role": "user", "content": "Hello"}
            ],
            "tools": [{
                "type": "function",
                "name": "shell",
                "description": "Run a shell command",
                "parameters": {"type": "object", "properties": {"command": {"type": "string"}}}
            }],
            "max_output_tokens": 8192,
        });

        let result = oai_to_chat(&oai);

        assert_eq!(result["model"], "zai-glm-5-2");
        assert_eq!(result["max_tokens"], 8192);
        assert_eq!(result["stream"], true);
        // System message is first.
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(result["messages"][0]["content"], "You are helpful.");
        assert_eq!(result["messages"][1]["role"], "user");
        assert_eq!(result["messages"][1]["content"], "Hello");
        assert_eq!(result["tools"][0]["function"]["name"], "shell");
        assert_eq!(
            result["tools"][0]["function"]["parameters"]["properties"]["command"]["type"],
            "string"
        );
    }

    #[test]
    fn test_function_call_roundtrip() {
        let oai = json!({
            "model": "zai-glm-5-2",
            "stream": true,
            "input": [
                {"type": "message", "role": "user", "content": "List files"},
                {"type": "function_call", "call_id": "call_123", "name": "shell", "arguments": "{\"command\":\"ls\"}"},
                {"type": "function_call_output", "call_id": "call_123", "output": "file1.txt\nfile2.txt"}
            ],
            "max_output_tokens": 16384,
        });

        let result = oai_to_chat(&oai);
        let messages = result["messages"]
            .as_array()
            .unwrap_or_else(|| unreachable!());

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_123");
        assert_eq!(messages[1]["tool_calls"][0]["type"], "function");
        assert_eq!(messages[1]["tool_calls"][0]["function"]["name"], "shell");
        assert_eq!(
            messages[1]["tool_calls"][0]["function"]["arguments"],
            "{\"command\":\"ls\"}"
        );
        assert_eq!(messages[2]["role"], "tool");
        assert_eq!(messages[2]["tool_call_id"], "call_123");
        assert_eq!(messages[2]["content"], "file1.txt\nfile2.txt");
    }

    #[test]
    fn test_non_streaming_response_translation() {
        let chat = json!({
            "id": "chatcmpl-abc",
            "model": "zai-glm-5-2",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "Hello back!",
                    "tool_calls": [{
                        "id": "call_456",
                        "type": "function",
                        "function": {
                            "name": "shell",
                            "arguments": "{\"command\":\"pwd\"}"
                        }
                    }]
                },
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });

        let result = chat_response_to_oai(&chat, "zai-glm-5-2");

        assert_eq!(result["object"], "response");
        assert_eq!(result["status"], "completed");
        assert_eq!(result["model"], "zai-glm-5-2");
        let output = result["output"]
            .as_array()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["text"], "Hello back!");
        assert_eq!(output[1]["type"], "function_call");
        assert_eq!(output[1]["name"], "shell");
        assert_eq!(output[1]["call_id"], "call_456");
        assert_eq!(result["usage"]["input_tokens"], 10,);
        assert_eq!(result["usage"]["output_tokens"], 5,);
    }

    #[test]
    fn block_list_content_maps_thinking_to_reasoning_and_text_to_message() {
        let chat = json!({
            "id": "chatcmpl-1",
            "model": "zai-glm-5-3",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "thinking", "thinking": [{"type": "text", "text": "The user is asking"}], "closed": true},
                        {"type": "text", "text": "pong zai-glm-5-3"}
                    ]
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}
        });

        let result = chat_response_to_oai(&chat, "zai-glm-5-3");

        assert_eq!(result["status"], "completed");
        let output = result["output"]
            .as_array()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(output.len(), 2);
        assert_eq!(output[0]["type"], "reasoning");
        assert_eq!(
            output[0]["content"],
            json!([{"type": "reasoning_text", "text": "The user is asking"}])
        );
        assert_eq!(output[0]["summary"], json!([]));
        assert!(output[0].get("encrypted_content").is_none());
        assert_eq!(output[1]["type"], "message");
        assert_eq!(
            output[1]["content"],
            json!([{"type": "output_text", "text": "pong zai-glm-5-3", "annotations": []}])
        );
    }

    #[test]
    fn unknown_block_types_in_block_content_are_ignored() {
        let chat = json!({
            "id": "chatcmpl-1",
            "model": "zai-glm-5-3",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "image", "url": "https://example.test/x.png"},
                        {"type": "text", "text": "only text survives"}
                    ]
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        });

        let result = chat_response_to_oai(&chat, "zai-glm-5-3");

        let output = result["output"]
            .as_array()
            .unwrap_or_else(|| unreachable!());
        assert_eq!(output.len(), 1);
        assert_eq!(output[0]["type"], "message");
        assert_eq!(output[0]["content"][0]["text"], "only text survives");
    }
}
