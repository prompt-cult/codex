#!/usr/bin/env python3
"""
zen_models_to_catalog.py

Reads the zen /models JSON response from stdin and writes a codex-rs
ModelInfo catalog JSON to stdout.

The output format matches the bundled models-manager/models.json schema.

Usage:
    curl -s https://opencode.ai/zen/v1/models \
        -H "Authorization: Bearer $OPENCODE_API_KEY" | \
        python3 zen_models_to_catalog.py > ~/.codex/zen_models.json

Model family detection (by slug prefix):
  gpt-*       -> GPT family  (Responses API, reasoning support, freeform patch)
  claude-*    -> Anthropic   (Responses API via proxy, extended thinking)
  gemini-*    -> Google      (Responses API via proxy)
  everything else -> generic OpenAI-compatible defaults
"""

import json
import sys


# Base instructions reused across all models — kept short since the zen proxy
# is provider-agnostic and each model has its own built-in system prompt.
BASE_INSTRUCTIONS = (
    "You are a coding agent. You and the user share the same workspace "
    "and collaborate to achieve the user's goals."
)

REASONING_LEVELS_FULL = [
    {"effort": "low",    "description": "Fast responses with lighter reasoning"},
    {"effort": "medium", "description": "Balances speed and reasoning depth"},
    {"effort": "high",   "description": "Greater reasoning depth for complex problems"},
    {"effort": "xhigh",  "description": "Maximum reasoning depth"},
]

REASONING_LEVELS_STANDARD = [
    {"effort": "low",    "description": "Fast responses with lighter reasoning"},
    {"effort": "medium", "description": "Balances speed and reasoning depth"},
    {"effort": "high",   "description": "Greater reasoning depth for complex problems"},
]


def family(slug: str) -> str:
    if slug.startswith("gpt-"):
        return "gpt"
    if slug.startswith("claude-"):
        return "claude"
    if slug.startswith("gemini-"):
        return "gemini"
    return "other"


def model_info(slug: str, priority: int) -> dict:
    fam = family(slug)

    # --- GPT family ---
    if fam == "gpt":
        is_codex = "codex" in slug
        is_mini_or_nano = any(x in slug for x in ("mini", "nano", "spark"))
        return {
            "slug": slug,
            "display_name": slug,
            "description": f"OpenAI {slug} via OpenCode Zen.",
            "base_instructions": BASE_INSTRUCTIONS,
            "visibility": "list",
            "supported_in_api": True,
            "priority": priority,
            "shell_type": "shell_command" if is_codex else "unified_exec",
            "apply_patch_tool_type": "freeform",
            "supports_parallel_tool_calls": True,
            "supports_reasoning_summaries": True,
            "default_reasoning_summary": "auto",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": REASONING_LEVELS_FULL if not is_mini_or_nano else REASONING_LEVELS_STANDARD,
            "support_verbosity": True,
            "default_verbosity": "low",
            "context_window": 400000 if is_codex else 272000,
            "auto_compact_token_limit": None,
            "effective_context_window_percent": 95,
            "truncation_policy": {"mode": "tokens", "limit": 10000},
            "input_modalities": ["text", "image"],
            "supports_image_detail_original": False,
            "supports_search_tool": False,
            "web_search_tool_type": "text",
            "experimental_supported_tools": [],
            "availability_nux": None,
            "upgrade": None,
            "model_messages": None,
        }

    # --- Claude family ---
    if fam == "claude":
        is_haiku = "haiku" in slug
        is_opus = "opus" in slug
        return {
            "slug": slug,
            "display_name": slug,
            "description": f"Anthropic {slug} via OpenCode Zen (proxied through Responses API).",
            "base_instructions": BASE_INSTRUCTIONS,
            "visibility": "list",
            "supported_in_api": True,
            "priority": priority,
            "shell_type": "shell_command",
            "apply_patch_tool_type": "freeform",
            "supports_parallel_tool_calls": False,
            "supports_reasoning_summaries": False,
            "default_reasoning_summary": "auto",
            "default_reasoning_level": "low" if is_haiku else ("high" if is_opus else "medium"),
            "supported_reasoning_levels": REASONING_LEVELS_STANDARD,
            "support_verbosity": False,
            "default_verbosity": None,
            "context_window": 200000,
            "auto_compact_token_limit": None,
            "effective_context_window_percent": 90,
            "truncation_policy": {"mode": "tokens", "limit": 10000},
            "input_modalities": ["text", "image"],
            "supports_image_detail_original": False,
            "supports_search_tool": False,
            "web_search_tool_type": "text",
            "experimental_supported_tools": [],
            "availability_nux": None,
            "upgrade": None,
            "model_messages": None,
        }

    # --- Gemini family ---
    if fam == "gemini":
        is_flash = "flash" in slug
        return {
            "slug": slug,
            "display_name": slug,
            "description": f"Google {slug} via OpenCode Zen.",
            "base_instructions": BASE_INSTRUCTIONS,
            "visibility": "list",
            "supported_in_api": True,
            "priority": priority,
            "shell_type": "shell_command",
            "apply_patch_tool_type": "freeform",
            "supports_parallel_tool_calls": True,
            "supports_reasoning_summaries": False,
            "default_reasoning_summary": "auto",
            "default_reasoning_level": "medium",
            "supported_reasoning_levels": REASONING_LEVELS_STANDARD,
            "support_verbosity": False,
            "default_verbosity": None,
            "context_window": 1000000 if not is_flash else 500000,
            "auto_compact_token_limit": None,
            "effective_context_window_percent": 90,
            "truncation_policy": {"mode": "tokens", "limit": 10000},
            "input_modalities": ["text", "image"],
            "supports_image_detail_original": False,
            "supports_search_tool": False,
            "web_search_tool_type": "text",
            "experimental_supported_tools": [],
            "availability_nux": None,
            "upgrade": None,
            "model_messages": None,
        }

    # --- Everything else (glm, minimax, kimi, qwen, etc.) ---
    return {
        "slug": slug,
        "display_name": slug,
        "description": f"{slug} via OpenCode Zen.",
        "base_instructions": BASE_INSTRUCTIONS,
        "visibility": "list",
        "supported_in_api": True,
        "priority": priority,
        "shell_type": "shell_command",
        "apply_patch_tool_type": "freeform",
        "supports_parallel_tool_calls": False,
        "supports_reasoning_summaries": False,
        "default_reasoning_summary": "auto",
        "default_reasoning_level": "medium",
        "supported_reasoning_levels": REASONING_LEVELS_STANDARD,
        "support_verbosity": False,
        "default_verbosity": None,
        "context_window": 128000,
        "auto_compact_token_limit": None,
        "effective_context_window_percent": 90,
        "truncation_policy": {"mode": "tokens", "limit": 10000},
        "input_modalities": ["text"],
        "supports_image_detail_original": False,
        "supports_search_tool": False,
        "web_search_tool_type": "text",
        "experimental_supported_tools": [],
        "availability_nux": None,
        "upgrade": None,
        "model_messages": None,
    }


# Priority order: GPT first (codex variants), then Claude, then Gemini, then others.
FAMILY_ORDER = {"gpt": 0, "claude": 1, "gemini": 2, "other": 3}

# Within each family, prefer larger/newer models first (simple heuristic:
# longer slug version numbers sort later numerically — so reverse sort).
def sort_key(slug: str):
    return (FAMILY_ORDER[family(slug)], slug)


def main():
    raw = sys.stdin.read()
    data = json.loads(raw)

    # zen /models returns {"data": [{"id": "...", "object": "model", ...}, ...]}
    slugs = [m["id"] for m in data.get("data", []) if m.get("id")]

    # filter out free/preview noise for now — keep paid + stable
    # (they still work via the proxy but show up in the picker)
    # Uncomment the filter below to hide free/preview models from the picker:
    # slugs = [s for s in slugs if not s.endswith("-free") and "preview" not in s]

    slugs_sorted = sorted(slugs, key=sort_key)

    models = [model_info(slug, priority=i) for i, slug in enumerate(slugs_sorted)]

    catalog = {"models": models}
    print(json.dumps(catalog, indent=2))


if __name__ == "__main__":
    main()
