#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = [
#   "httpx>=0.27",
#   "fastapi>=0.111",
#   "uvicorn>=0.29",
#   "python-dotenv>=1.0",
# ]
# ///
"""
zen_proxy.py — OpenCode Zen sidecar proxy for codex-rs.

Translates between the OpenAI Responses API (what codex-rs speaks) and the
correct zen endpoint for each model family:

  GPT models   → /zen/v1/responses      (native OAI Responses, passthrough)
  Claude models→ /zen/v1/messages       (Anthropic Messages API, full translation)
  Others       → stubbed 501 for now

Translation layers
------------------
GPT: straight passthrough, no changes needed.

Claude (request):
  OAI Responses body  →  Anthropic Messages body
  - input[]           →  messages[]  (role/content mapping)
  - instructions      →  system
  - tools[].parameters→  tools[].input_schema
  - drop: store, prompt_cache_key, client_metadata, include, reasoning,
          parallel_tool_calls, tool_choice "auto" → keep as-is

Claude (response SSE):
  Anthropic SSE events →  OAI Responses SSE events
  Text path:
    message_start              → response.created + response.in_progress +
                                  response.output_item.added +
                                  response.content_part.added
    content_block_delta (text) → response.output_text.delta
    message_delta/stop         → response.output_text.done +
                                  response.content_part.done +
                                  response.output_item.done +
                                  response.completed
  Tool-use path:
    content_block_start (tool_use) → response.output_item.added (function_call)
    content_block_delta (input_json_delta) → response.output_item.added (accumulate)
    content_block_stop             → response.function_call_arguments.done
    message_delta stop_reason=tool_use → response.completed with function_call output

Multi-turn tool result (OAI → Anthropic):
  input item type=function_call_output → messages role=user content=[tool_result]

Env vars (loaded from .env in the script directory when present):
  OPENCODE_API_KEY   — zen bearer / x-api-key token
  ZEN_PROXY_PORT     — port (default: 9099)
  ZEN_BASE_URL       — override zen base (default: https://opencode.ai/zen/v1)
  ZEN_LOG_LEVEL      — DEBUG|INFO|WARNING (default: DEBUG)
"""

import json
import logging
import os
import sys
import time
import uuid
from pathlib import Path
from typing import Any

import httpx
from dotenv import load_dotenv
from fastapi import FastAPI, Request, Response
from fastapi.responses import StreamingResponse
import uvicorn

# ── env ───────────────────────────────────────────────────────────────────────
_ENV_PATH = Path(__file__).resolve().parent / ".env"
if _ENV_PATH.exists():
    load_dotenv(_ENV_PATH)

ZEN_BASE_URL = os.environ.get("ZEN_BASE_URL", "https://opencode.ai/zen/v1")
ZEN_PROXY_PORT = int(os.environ.get("ZEN_PROXY_PORT", "9099"))
LOG_LEVEL = os.environ.get("ZEN_LOG_LEVEL", "DEBUG").upper()

# ── logging ───────────────────────────────────────────────────────────────────
logging.basicConfig(
    level=getattr(logging, LOG_LEVEL, logging.DEBUG),
    format="%(asctime)s [%(levelname)s] %(name)s — %(message)s",
    handlers=[logging.StreamHandler(sys.stderr)],
)
log = logging.getLogger("zen_proxy")

# ── model routing ─────────────────────────────────────────────────────────────
def is_claude(model: str) -> bool:
    return model.startswith("claude-")

def is_gpt(model: str) -> bool:
    return model.startswith("gpt-")

# ── OAI Responses → Anthropic Messages translation ───────────────────────────

def _oai_tool_to_anthropic(tool: dict) -> dict | None:
    """Convert one OAI Responses tool definition to Anthropic format.
    Returns None if the tool cannot be represented (empty name etc)."""
    # OAI Responses tools can be:
    #   {"type":"function","name":"foo","description":"...","parameters":{...}}   (flat)
    #   {"type":"function","function":{"name":"foo","description":"...","parameters":{...}}}  (nested)
    if tool.get("type") == "function":
        nested = tool.get("function")
        if nested:
            name = nested.get("name", "")
            description = nested.get("description", "")
            parameters = nested.get("parameters", {"type": "object", "properties": {}})
        else:
            name = tool.get("name", "")
            description = tool.get("description", "")
            parameters = tool.get("parameters", {"type": "object", "properties": {}})
    else:
        # custom/computer_use/etc — skip
        name = tool.get("name", "")
        description = tool.get("description", "")
        parameters = tool.get("parameters", {"type": "object", "properties": {}})

    if not name:
        log.debug("skipping tool with empty name: %s", tool)
        return None

    return {
        "name": name,
        "description": description,
        "input_schema": parameters,
    }


def _oai_input_to_anthropic_messages(input_items: list) -> tuple[str | None, list]:
    """
    Convert OAI Responses `input` array to (system_prompt, messages[]).

    OAI item types we handle:
      {"type":"message","role":"user/assistant","content":[{"type":"input_text","text":"..."}]}
      {"type":"message","role":"user/assistant","content": "string"}
      {"type":"function_call","name":"...","arguments":"...","call_id":"..."}
      {"type":"function_call_output","call_id":"...","output":"..."}
    """
    messages = []
    system = None

    for item in input_items:
        itype = item.get("type", "message")
        role = item.get("role", "user")

        if itype == "message":
            content_raw = item.get("content", "")
            if isinstance(content_raw, str):
                text = content_raw
            elif isinstance(content_raw, list):
                parts = []
                for part in content_raw:
                    if part.get("type") in ("input_text", "text"):
                        parts.append(part.get("text", ""))
                    elif part.get("type") == "image_url":
                        # basic image passthrough — Anthropic format
                        parts.append({"type": "image", "source": {"type": "url", "url": part["image_url"]["url"]}})
                text = "\n".join(p if isinstance(p, str) else str(p) for p in parts) if parts else ""
            else:
                text = str(content_raw)

            # Anthropic only knows "user" and "assistant"; map system/developer → system prompt
            if role in ("system", "developer"):
                system = (system + "\n\n" + text) if system else text
            else:
                messages.append({"role": role, "content": text})

        elif itype == "function_call":
            # Assistant produced a tool call — represents as assistant message with tool_use
            call_id = item.get("call_id", item.get("id", f"call_{uuid.uuid4().hex[:8]}"))
            try:
                inp = json.loads(item.get("arguments", "{}"))
            except Exception:
                inp = {}
            # Find last assistant message to append to, or create new
            if messages and messages[-1]["role"] == "assistant":
                last = messages[-1]
                if isinstance(last["content"], str):
                    last["content"] = [{"type": "text", "text": last["content"]}] if last["content"] else []
                last["content"].append({
                    "type": "tool_use",
                    "id": call_id,
                    "name": item.get("name", ""),
                    "input": inp,
                })
            else:
                messages.append({
                    "role": "assistant",
                    "content": [{
                        "type": "tool_use",
                        "id": call_id,
                        "name": item.get("name", ""),
                        "input": inp,
                    }]
                })

        elif itype == "function_call_output":
            # Tool result — user message with tool_result
            call_id = item.get("call_id", "")
            output = item.get("output", "")
            messages.append({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": call_id,
                    "content": output,
                }]
            })

    return system, messages


def oai_to_anthropic_body(oai_body: dict) -> dict:
    """Convert full OAI Responses request body to Anthropic Messages body."""
    model = oai_body.get("model", "")
    is_stream = oai_body.get("stream", False)

    system_from_instructions = oai_body.get("instructions") or None
    input_items = oai_body.get("input", [])

    system_from_input, messages = _oai_input_to_anthropic_messages(input_items)
    system = system_from_instructions or system_from_input

    oai_tools = oai_body.get("tools", [])
    anthropic_tools = [t for t in (_oai_tool_to_anthropic(x) for x in oai_tools) if t is not None] if oai_tools else []

    body: dict[str, Any] = {
        "model": model,
        "max_tokens": oai_body.get("max_output_tokens") or 16384,
        "messages": messages,
        "stream": is_stream,
    }
    if system:
        body["system"] = system
    if anthropic_tools:
        body["tools"] = anthropic_tools

    log.debug("anthropic body: %s", json.dumps(body, indent=2))
    return body


# ── Anthropic SSE → OAI Responses SSE translation ────────────────────────────

def _sse_event(event_type: str, data: dict) -> str:
    return f"event: {event_type}\ndata: {json.dumps(data)}\n\n"


async def anthropic_to_oai_sse(model: str, line_iter) -> "AsyncIterator[str]":
    """
    Translate Anthropic streaming SSE → OAI Responses SSE.

    Handles both text responses and tool_use (function call) responses.
    Multi-block responses (text + tool) are supported.
    """
    resp_id = f"resp_{uuid.uuid4().hex}"
    created_at = int(time.time())
    seq = 0

    # State per content block
    blocks: dict[int, dict] = {}   # index → block state
    usage = None
    stop_reason = None

    # Emit opening envelope once we see message_start
    opened = False

    def _skeleton():
        return {
            "id": resp_id, "object": "response", "created_at": created_at,
            "status": "in_progress", "model": model, "output": [], "usage": None, "error": None,
        }

    async for raw_line in line_iter:
        line = raw_line if isinstance(raw_line, str) else raw_line.decode("utf-8", errors="replace")
        line = line.rstrip()
        log.debug("[anthropic-raw] %s", line)

        if not line.startswith("data:"):
            continue
        payload_str = line[5:].strip()
        if not payload_str or payload_str == "[DONE]":
            continue
        try:
            ev = json.loads(payload_str)
        except json.JSONDecodeError:
            log.warning("non-JSON anthropic SSE: %s", payload_str)
            continue

        etype = ev.get("type", "")

        # ── message_start ────────────────────────────────────────────────────
        if etype == "message_start":
            if not opened:
                opened = True
                sk = _skeleton()
                yield _sse_event("response.created", {**sk})
                yield _sse_event("response.in_progress", {**sk})
                seq = 2

        # ── content_block_start ──────────────────────────────────────────────
        elif etype == "content_block_start":
            idx = ev.get("index", 0)
            block = ev.get("content_block", {})
            btype = block.get("type", "text")
            blocks[idx] = {"type": btype, "text": "", "tool_id": block.get("id", ""), "tool_name": block.get("name", ""), "json_acc": ""}

            if btype == "text":
                msg_id = f"msg_{uuid.uuid4().hex}"
                blocks[idx]["msg_id"] = msg_id
                yield _sse_event("response.output_item.added", {
                    "type": "response.output_item.added", "output_index": idx,
                    "item": {"id": msg_id, "type": "message", "status": "in_progress", "role": "assistant", "content": []},
                    "sequence_number": seq,
                })
                seq += 1
                yield _sse_event("response.content_part.added", {
                    "type": "response.content_part.added", "output_index": idx, "content_index": 0,
                    "item_id": msg_id, "part": {"type": "output_text", "text": "", "annotations": []},
                    "sequence_number": seq,
                })
                seq += 1

            elif btype == "tool_use":
                call_id = block.get("id", f"call_{uuid.uuid4().hex[:8]}")
                blocks[idx]["call_id"] = call_id
                # Emit function_call item as in_progress
                yield _sse_event("response.output_item.added", {
                    "type": "response.output_item.added", "output_index": idx,
                    "item": {
                        "id": call_id, "type": "function_call", "status": "in_progress",
                        "name": block.get("name", ""), "arguments": "",
                        "call_id": call_id,
                    },
                    "sequence_number": seq,
                })
                seq += 1

        # ── content_block_delta ──────────────────────────────────────────────
        elif etype == "content_block_delta":
            idx = ev.get("index", 0)
            delta = ev.get("delta", {})
            dtype = delta.get("type", "")
            block = blocks.get(idx, {})

            if dtype == "text_delta":
                text = delta.get("text", "")
                block["text"] = block.get("text", "") + text
                msg_id = block.get("msg_id", "")
                yield _sse_event("response.output_text.delta", {
                    "type": "response.output_text.delta", "output_index": idx, "content_index": 0,
                    "item_id": msg_id, "delta": text, "sequence_number": seq,
                })
                seq += 1

            elif dtype == "input_json_delta":
                chunk = delta.get("partial_json", "")
                block["json_acc"] = block.get("json_acc", "") + chunk
                call_id = block.get("call_id", "")
                # stream argument deltas
                yield _sse_event("response.function_call_arguments.delta", {
                    "type": "response.function_call_arguments.delta",
                    "output_index": idx, "item_id": call_id,
                    "delta": chunk, "sequence_number": seq,
                })
                seq += 1

        # ── content_block_stop ───────────────────────────────────────────────
        elif etype == "content_block_stop":
            idx = ev.get("index", 0)
            block = blocks.get(idx, {})
            btype = block.get("type", "text")

            if btype == "text":
                msg_id = block.get("msg_id", "")
                full_text = block.get("text", "")
                yield _sse_event("response.output_text.done", {
                    "type": "response.output_text.done", "output_index": idx, "content_index": 0,
                    "item_id": msg_id, "text": full_text, "sequence_number": seq,
                })
                seq += 1
                yield _sse_event("response.content_part.done", {
                    "type": "response.content_part.done", "output_index": idx, "content_index": 0,
                    "item_id": msg_id, "part": {"type": "output_text", "text": full_text, "annotations": []},
                    "sequence_number": seq,
                })
                seq += 1
                yield _sse_event("response.output_item.done", {
                    "type": "response.output_item.done", "output_index": idx,
                    "item": {"id": msg_id, "type": "message", "status": "completed", "role": "assistant",
                             "content": [{"type": "output_text", "text": full_text, "annotations": []}]},
                    "sequence_number": seq,
                })
                seq += 1

            elif btype == "tool_use":
                call_id = block.get("call_id", "")
                tool_name = block.get("tool_name", "")
                arguments = block.get("json_acc", "")
                yield _sse_event("response.function_call_arguments.done", {
                    "type": "response.function_call_arguments.done",
                    "output_index": idx, "item_id": call_id,
                    "arguments": arguments, "sequence_number": seq,
                })
                seq += 1
                yield _sse_event("response.output_item.done", {
                    "type": "response.output_item.done", "output_index": idx,
                    "item": {
                        "id": call_id, "type": "function_call", "status": "completed",
                        "name": tool_name, "arguments": arguments, "call_id": call_id,
                    },
                    "sequence_number": seq,
                })
                seq += 1

        # ── message_delta ────────────────────────────────────────────────────
        elif etype == "message_delta":
            delta = ev.get("delta", {})
            stop_reason = delta.get("stop_reason")
            usage_delta = ev.get("usage", {})
            usage = {
                "input_tokens": usage_delta.get("input_tokens", 0),
                "output_tokens": usage_delta.get("output_tokens", 0),
                "total_tokens": usage_delta.get("input_tokens", 0) + usage_delta.get("output_tokens", 0),
            }

        # ── message_stop ─────────────────────────────────────────────────────
        elif etype == "message_stop":
            # Build output array from blocks
            output = []
            for idx in sorted(blocks.keys()):
                block = blocks[idx]
                if block["type"] == "text":
                    output.append({
                        "id": block.get("msg_id", f"msg_{uuid.uuid4().hex}"),
                        "type": "message", "status": "completed", "role": "assistant",
                        "content": [{"type": "output_text", "text": block.get("text", ""), "annotations": []}],
                    })
                elif block["type"] == "tool_use":
                    output.append({
                        "id": block.get("call_id", f"call_{uuid.uuid4().hex[:8]}"),
                        "type": "function_call", "status": "completed",
                        "name": block.get("tool_name", ""),
                        "arguments": block.get("json_acc", ""),
                        "call_id": block.get("call_id", ""),
                    })

            # IMPORTANT: codex-rs ResponsesStreamEvent parses `event.response` as the
            # nested value under the "response" key — must be wrapped:
            # {"type":"response.completed","response":{"id":...,"usage":...}}
            completed_response = {
                "id": resp_id,
                "object": "response",
                "created_at": created_at,
                "status": "completed",
                "model": model,
                "output": output,
                "usage": {
                    "input_tokens": (usage or {}).get("input_tokens", 0),
                    "input_tokens_details": {"cached_tokens": 0},
                    "output_tokens": (usage or {}).get("output_tokens", 0),
                    "output_tokens_details": {"reasoning_tokens": 0},
                    "total_tokens": (usage or {}).get("total_tokens", 0),
                },
                "error": None,
            }
            yield _sse_event("response.completed", {
                "type": "response.completed",
                "response": completed_response,
            })

        elif etype == "ping":
            cost = ev.get("cost")
            log.info("[zen] ping cost=%s", cost)
            yield _sse_event("ping", ev)

        else:
            log.debug("[anthropic unhandled] %s", etype)


# ── GPT sparse SSE normaliser (claude via /responses - not used now but kept) ─

async def _normalise_sparse_sse(model: str, line_iter) -> "AsyncIterator[str]":
    """Pads the minimal zen /responses SSE for Claude into full OAI shape."""
    resp_id = f"resp_{uuid.uuid4().hex}"
    msg_id = f"msg_{uuid.uuid4().hex}"
    skeleton = {
        "id": resp_id, "object": "response", "created_at": int(time.time()),
        "status": "in_progress", "model": model, "output": [], "usage": None, "error": None,
    }
    yield _sse_event("response.created", {**skeleton})
    yield _sse_event("response.in_progress", {**skeleton})
    yield _sse_event("response.output_item.added", {
        "type": "response.output_item.added", "output_index": 0,
        "item": {"id": msg_id, "type": "message", "status": "in_progress", "role": "assistant", "content": []},
    })
    yield _sse_event("response.content_part.added", {
        "type": "response.content_part.added", "output_index": 0, "content_index": 0,
        "item_id": msg_id, "part": {"type": "output_text", "text": "", "annotations": []},
    })

    full_text = ""
    usage = None
    seq = 4

    async for raw_line in line_iter:
        line = raw_line if isinstance(raw_line, str) else raw_line.decode("utf-8", errors="replace")
        line = line.rstrip()
        if not line.startswith("data:"):
            continue
        payload_str = line[5:].strip()
        if not payload_str:
            continue
        try:
            payload = json.loads(payload_str)
        except json.JSONDecodeError:
            continue

        etype = payload.get("type", "")
        if etype == "response.output_text.delta":
            delta = payload.get("delta", "")
            full_text += delta
            yield _sse_event("response.output_text.delta", {
                "type": "response.output_text.delta", "output_index": 0, "content_index": 0,
                "item_id": msg_id, "delta": delta, "sequence_number": seq,
            })
            seq += 1
        elif etype == "response.completed":
            usage = payload.get("response", {}).get("usage")
            yield _sse_event("response.output_text.done", {"type": "response.output_text.done", "output_index": 0, "content_index": 0, "item_id": msg_id, "text": full_text, "sequence_number": seq})
            seq += 1
            yield _sse_event("response.content_part.done", {"type": "response.content_part.done", "output_index": 0, "content_index": 0, "item_id": msg_id, "part": {"type": "output_text", "text": full_text, "annotations": []}, "sequence_number": seq})
            seq += 1
            yield _sse_event("response.output_item.done", {"type": "response.output_item.done", "output_index": 0, "item": {"id": msg_id, "type": "message", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": full_text, "annotations": []}]}, "sequence_number": seq})
            seq += 1
            yield _sse_event("response.completed", {
                "type": "response.completed",
                "response": {
                    "id": resp_id, "object": "response", "created_at": int(time.time()),
                    "status": "completed", "model": model,
                    "output": [{"id": msg_id, "type": "message", "status": "completed", "role": "assistant", "content": [{"type": "output_text", "text": full_text, "annotations": []}]}],
                    "usage": {
                        "input_tokens": (usage or {}).get("input_tokens", 0),
                        "input_tokens_details": {"cached_tokens": 0},
                        "output_tokens": (usage or {}).get("output_tokens", 0),
                        "output_tokens_details": {"reasoning_tokens": 0},
                        "total_tokens": (usage or {}).get("total_tokens", 0),
                    },
                    "error": None,
                },
            })
        elif etype == "ping":
            log.info("[zen] ping cost=%s", payload.get("cost"))
            yield _sse_event("ping", payload)


# ── FastAPI ───────────────────────────────────────────────────────────────────
app = FastAPI(title="zen_proxy", version="0.2.0")


def _get_api_key(request: Request) -> str:
    auth = request.headers.get("authorization", "")
    if auth.lower().startswith("bearer "):
        return auth[7:].strip()
    return os.environ.get("OPENCODE_API_KEY", "")


@app.get("/health")
async def health():
    return {"status": "ok", "proxy": "zen_proxy", "version": "0.2.0", "upstream": ZEN_BASE_URL}


@app.get("/v1/models")
async def proxy_models(request: Request):
    api_key = _get_api_key(request)
    async with httpx.AsyncClient(timeout=30.0) as client:
        resp = await client.get(f"{ZEN_BASE_URL}/models", headers={"Authorization": f"Bearer {api_key}"})
    return Response(content=resp.content, status_code=resp.status_code, media_type="application/json")


@app.post("/v1/responses")
async def proxy_responses(request: Request):
    body_bytes = await request.body()
    try:
        body = json.loads(body_bytes)
    except json.JSONDecodeError:
        return Response(content="bad json body", status_code=400)

    model: str = body.get("model", "")
    is_stream: bool = body.get("stream", False)
    api_key = _get_api_key(request)

    log.info("→ model=%s stream=%s", model, is_stream)
    log.debug("→ body: %s", json.dumps(body, indent=2))

    if not api_key:
        return Response(content='{"error":"no api key"}', status_code=401, media_type="application/json")

    # ── route by model family ─────────────────────────────────────────────────
    if is_claude(model):
        return await _handle_claude(request, body, model, is_stream, api_key)
    elif is_gpt(model):
        return await _handle_gpt(body_bytes, body, model, is_stream, api_key, request)
    else:
        log.warning("unrouted model=%s", model)
        return Response(
            content=json.dumps({"error": {"message": f"zen_proxy: no route for model '{model}'", "type": "proxy_not_implemented"}}),
            status_code=501, media_type="application/json",
        )


# ── GPT handler: passthrough to /responses ────────────────────────────────────

async def _handle_gpt(body_bytes: bytes, body: dict, model: str, is_stream: bool, api_key: str, request: Request):
    upstream_url = f"{ZEN_BASE_URL}/responses"
    headers = {
        "Authorization": f"Bearer {api_key}",
        "Content-Type": "application/json",
        "Accept": "text/event-stream" if is_stream else "application/json",
    }
    for h, v in request.headers.items():
        if h.lower().startswith("x-codex-") or h.lower() in ("x-client-request-id", "session_id"):
            headers[h] = v

    log.info("→ GPT passthrough url=%s", upstream_url)

    if not is_stream:
        async with httpx.AsyncClient(timeout=120.0) as client:
            resp = await client.post(upstream_url, headers=headers, content=body_bytes)
        return Response(content=resp.content, status_code=resp.status_code,
                        media_type=resp.headers.get("content-type", "application/json"))

    async def stream_gen():
        async with httpx.AsyncClient(timeout=httpx.Timeout(120.0, connect=10.0)) as client:
            async with client.stream("POST", upstream_url, headers=headers, content=body_bytes) as resp:
                log.info("← GPT upstream status=%d", resp.status_code)
                if resp.status_code != 200:
                    err = await resp.aread()
                    log.error("← GPT upstream error: %s", err.decode())
                    yield f"data: {json.dumps({'error': {'message': err.decode()}})}\n\n"
                    return
                async for line in resp.aiter_lines():
                    log.debug("[gpt-raw] %s", line)
                    yield line + "\n"

    return StreamingResponse(stream_gen(), media_type="text/event-stream",
                             headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"})


# ── Claude handler: translate to /messages ────────────────────────────────────

async def _handle_claude(request: Request, body: dict, model: str, is_stream: bool, api_key: str):
    upstream_url = f"{ZEN_BASE_URL}/messages"
    anthropic_body = oai_to_anthropic_body(body)

    headers = {
        "x-api-key": api_key,
        "Content-Type": "application/json",
        "anthropic-version": "2023-06-01",
        "Accept": "text/event-stream" if is_stream else "application/json",
    }

    log.info("→ Claude /messages url=%s stream=%s", upstream_url, is_stream)

    # ── non-streaming ─────────────────────────────────────────────────────────
    if not is_stream:
        async with httpx.AsyncClient(timeout=120.0) as client:
            resp = await client.post(upstream_url, headers=headers,
                                     content=json.dumps(anthropic_body))
        log.info("← Claude upstream status=%d", resp.status_code)
        log.debug("← Claude body: %s", resp.text[:500])

        if resp.status_code != 200:
            return Response(content=resp.content, status_code=resp.status_code, media_type="application/json")

        # Convert Anthropic response → OAI Responses response
        ant = resp.json()
        resp_id = f"resp_{uuid.uuid4().hex}"
        output_text = ""
        output = []
        for block in ant.get("content", []):
            if block.get("type") == "text":
                msg_id = f"msg_{uuid.uuid4().hex}"
                output_text = block.get("text", "")
                output.append({
                    "id": msg_id, "type": "message", "status": "completed", "role": "assistant",
                    "content": [{"type": "output_text", "text": output_text, "annotations": []}],
                })
            elif block.get("type") == "tool_use":
                call_id = block.get("id", f"call_{uuid.uuid4().hex[:8]}")
                output.append({
                    "id": call_id, "type": "function_call", "status": "completed",
                    "name": block.get("name", ""), "call_id": call_id,
                    "arguments": json.dumps(block.get("input", {})),
                })

        usage_raw = ant.get("usage", {})
        oai_response = {
            "id": resp_id, "object": "response", "created_at": int(time.time()),
            "status": "completed", "model": model, "output": output,
            "usage": {
                "input_tokens": usage_raw.get("input_tokens", 0),
                "output_tokens": usage_raw.get("output_tokens", 0),
                "total_tokens": usage_raw.get("input_tokens", 0) + usage_raw.get("output_tokens", 0),
            },
            "error": None,
        }
        return Response(content=json.dumps(oai_response), status_code=200, media_type="application/json")

    # ── streaming ─────────────────────────────────────────────────────────────
    async def stream_gen():
        async with httpx.AsyncClient(timeout=httpx.Timeout(300.0, connect=10.0)) as client:
            async with client.stream("POST", upstream_url, headers=headers,
                                     content=json.dumps(anthropic_body)) as resp:
                log.info("← Claude upstream stream status=%d", resp.status_code)
                if resp.status_code != 200:
                    err = await resp.aread()
                    log.error("← Claude upstream error: %s", err.decode())
                    yield f"data: {json.dumps({'error': {'message': err.decode()}})}\n\n"
                    return
                async for chunk in anthropic_to_oai_sse(model, resp.aiter_lines()):
                    log.debug("← translated: %s", chunk.rstrip())
                    yield chunk

    return StreamingResponse(stream_gen(), media_type="text/event-stream",
                             headers={"Cache-Control": "no-cache", "X-Accel-Buffering": "no"})


# ── entrypoint ────────────────────────────────────────────────────────────────
if __name__ == "__main__":
    pid = os.getpid()
    pid_file = Path("/tmp/zen_proxy.pid")
    pid_file.write_text(str(pid))
    log.info("zen_proxy v0.2.0 starting pid=%d port=%d upstream=%s", pid, ZEN_PROXY_PORT, ZEN_BASE_URL)
    try:
        uvicorn.run(app, host="127.0.0.1", port=ZEN_PROXY_PORT,
                    log_level=LOG_LEVEL.lower(), access_log=True)
    finally:
        pid_file.unlink(missing_ok=True)
        log.info("zen_proxy stopped pid=%d", pid)
