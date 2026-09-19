//! Model metadata types used by the translating proxies, vendored from the
//! former `codex-protocol::openai_models` + `config_types` modules.
//!
//! This is the minimal reachable subset: exactly what `codex-mistral-proxy`
//! needs to emit a `ModelsResponse` the codex `ModelsClient` deserialises.
//! Everything else in the original protocol crate (ModelPreset, ModelUpgrade,
//! personality templates, ClientVersion, TS/JsonSchema derive machinery) was
//! unreachable from the proxies and is gone; see the
//! `broken-mistral-introspection-pre-slimdown` tag in history.

use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

/// A summary of the reasoning performed by the model.
#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningSummary {
    #[default]
    Auto,
    Concise,
    Detailed,
    /// Option to disable reasoning summaries.
    None,
}

/// Controls output length/detail on GPT-5 models via the Responses API.
#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    Low,
    #[default]
    Medium,
    High,
}

/// See https://platform.openai.com/docs/guides/reasoning?api-mode=responses#get-started-with-reasoning
#[derive(Debug, Serialize, Deserialize, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    XHigh,
}

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_value(serde_json::Value::String(s.to_string()))
            .map_err(|_| format!("invalid reasoning_effort: {s}"))
    }
}

/// Canonical user-input modality tags advertised by a model.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum InputModality {
    /// Plain text turns and tool payloads.
    Text,
    /// Image attachments included in user turns.
    Image,
}

/// Backward-compatible default when `input_modalities` is omitted on the wire.
pub fn default_input_modalities() -> Vec<InputModality> {
    vec![InputModality::Text, InputModality::Image]
}

/// A reasoning effort option that can be surfaced for a model.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReasoningEffortPreset {
    /// Effort level that the model supports.
    pub effort: ReasoningEffort,
    /// Short human description shown next to the effort in UIs.
    pub description: String,
}

/// Visibility of a model in the picker or APIs.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ModelVisibility {
    List,
    Hide,
    None,
}

/// Shell execution capability for a model.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConfigShellToolType {
    Default,
    Local,
    UnifiedExec,
    Disabled,
    ShellCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ApplyPatchToolType {
    Freeform,
    Function,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchToolType {
    #[default]
    Text,
    TextAndImage,
}

/// Server-provided truncation policy metadata for a model.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TruncationMode {
    Bytes,
    Tokens,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct TruncationPolicyConfig {
    pub mode: TruncationMode,
    pub limit: i64,
}

impl TruncationPolicyConfig {
    pub const fn bytes(limit: i64) -> Self {
        Self {
            mode: TruncationMode::Bytes,
            limit,
        }
    }

    pub const fn tokens(limit: i64) -> Self {
        Self {
            mode: TruncationMode::Tokens,
            limit,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelInfoUpgrade {
    pub model: String,
    pub migration_markdown: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ModelAvailabilityNux {
    pub message: String,
}

const fn default_effective_context_window_percent() -> i64 {
    95
}

/// Model metadata returned by the Codex backend `/models` endpoint.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelInfo {
    pub slug: String,
    pub display_name: String,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_level: Option<ReasoningEffort>,
    pub supported_reasoning_levels: Vec<ReasoningEffortPreset>,
    pub shell_type: ConfigShellToolType,
    pub visibility: ModelVisibility,
    pub supported_in_api: bool,
    pub priority: i32,
    #[serde(default)]
    pub additional_speed_tiers: Vec<String>,
    pub availability_nux: Option<ModelAvailabilityNux>,
    pub upgrade: Option<ModelInfoUpgrade>,
    pub base_instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_messages: Option<ModelMessages>,
    pub supports_reasoning_summaries: bool,
    #[serde(default)]
    pub default_reasoning_summary: ReasoningSummary,
    pub support_verbosity: bool,
    pub default_verbosity: Option<Verbosity>,
    pub apply_patch_tool_type: Option<ApplyPatchToolType>,
    #[serde(default)]
    pub web_search_tool_type: WebSearchToolType,
    pub truncation_policy: TruncationPolicyConfig,
    pub supports_parallel_tool_calls: bool,
    #[serde(default)]
    pub supports_image_detail_original: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    /// Token threshold for automatic compaction. When omitted, core derives it
    /// from `context_window` (90%). When provided, core clamps it to 90% of the
    /// context window when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_token_limit: Option<i64>,
    /// Percentage of the context window considered usable for inputs, after
    /// reserving headroom for system prompts, tool overhead, and model output.
    #[serde(default = "default_effective_context_window_percent")]
    pub effective_context_window_percent: i64,
    pub experimental_supported_tools: Vec<String>,
    /// Input modalities accepted by the backend for this model.
    #[serde(default = "default_input_modalities")]
    pub input_modalities: Vec<InputModality>,
    /// Internal-only marker set by core when a model slug resolved to fallback metadata.
    #[serde(default, skip_serializing, skip_deserializing)]
    pub used_fallback_model_metadata: bool,
    #[serde(default)]
    pub supports_search_tool: bool,
}

impl ModelInfo {
    pub fn auto_compact_token_limit(&self) -> Option<i64> {
        let context_limit = self
            .context_window
            .map(|context_window| (context_window * 9) / 10);
        let config_limit = self.auto_compact_token_limit;
        if let Some(context_limit) = context_limit {
            return Some(
                config_limit.map_or(context_limit, |limit| std::cmp::min(limit, context_limit)),
            );
        }
        config_limit
    }
}

/// A strongly-typed template for assembling model instructions and developer
/// messages. If instructions_* is populated and valid, it overrides
/// base_instructions.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelMessages {
    pub instructions_template: Option<String>,
    pub instructions_variables: Option<ModelInstructionsVariables>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ModelInstructionsVariables {
    pub personality_default: Option<String>,
    pub personality_friendly: Option<String>,
    pub personality_pragmatic: Option<String>,
}

/// Response wrapper for `/models`.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq, Default)]
pub struct ModelsResponse {
    pub models: Vec<ModelInfo>,
}
