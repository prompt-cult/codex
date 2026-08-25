//! Translate Mistral's raw `/models` response into the codex `ModelsResponse`
//! shape (`{"models":[ModelInfo,…]}`) that `codex-api` deserializes.
//!
//! Only chat-capable models are surfaced; embeddings, moderation, OCR, and
//! other non-chat models Mistral lists are filtered out.

use anyhow::Context;
use anyhow::Result;
use codex_protocol::config_types::ReasoningSummary;
use codex_protocol::openai_models::ConfigShellToolType;
use codex_protocol::openai_models::ModelInfo;
use codex_protocol::openai_models::ModelVisibility;
use codex_protocol::openai_models::ModelsResponse;
use codex_protocol::openai_models::TruncationPolicyConfig;
use codex_protocol::openai_models::WebSearchToolType;
use codex_protocol::openai_models::default_input_modalities;
use serde::Deserialize;

/// Fallback context window used when Mistral omits `max_context_length`.
const DEFAULT_CONTEXT_WINDOW: i64 = 128_000;
/// Byte budget for input truncation; overridable per-model via config.
const DEFAULT_TRUNCATION_BYTES: i64 = 10_000;

/// Top-level Mistral `/models` envelope. Extra fields are ignored.
#[derive(Debug, Deserialize)]
struct MistralModelList {
    #[serde(default)]
    data: Vec<MistralModel>,
}

/// A single Mistral model entry. Only the fields we map are declared; all are
/// tolerant of omission so upstream schema drift does not hard-fail discovery.
#[derive(Debug, Deserialize)]
struct MistralModel {
    id: String,
    #[serde(default)]
    capabilities: MistralCapabilities,
    #[serde(default)]
    max_context_length: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
struct MistralCapabilities {
    #[serde(default)]
    completion_chat: bool,
}

/// Parse raw Mistral `/models` JSON and build a codex [`ModelsResponse`].
pub fn translate_mistral_models(raw: &[u8]) -> Result<ModelsResponse> {
    let list: MistralModelList =
        serde_json::from_slice(raw).context("parsing Mistral /models response")?;

    let models = list
        .data
        .into_iter()
        .filter(|m| m.capabilities.completion_chat)
        .enumerate()
        .map(|(index, m)| model_info_for(index, m))
        .collect();

    Ok(ModelsResponse { models })
}

/// Build a fully-populated [`ModelInfo`] for a chat-capable Mistral model.
///
/// `ModelInfo` has no `Default`, so every field is set explicitly. Optional and
/// reasoning-related metadata Mistral does not provide is left empty/`None`;
/// users can override context window and related limits via config.
fn model_info_for(index: usize, m: MistralModel) -> ModelInfo {
    ModelInfo {
        slug: m.id.clone(),
        display_name: m.id,
        description: None,
        default_reasoning_level: None,
        supported_reasoning_levels: Vec::new(),
        shell_type: ConfigShellToolType::ShellCommand,
        visibility: ModelVisibility::List,
        supported_in_api: true,
        priority: index as i32,
        additional_speed_tiers: Vec::new(),
        availability_nux: None,
        upgrade: None,
        base_instructions: String::new(),
        model_messages: None,
        supports_reasoning_summaries: false,
        default_reasoning_summary: ReasoningSummary::Auto,
        support_verbosity: false,
        default_verbosity: None,
        apply_patch_tool_type: None,
        web_search_tool_type: WebSearchToolType::Text,
        truncation_policy: TruncationPolicyConfig::bytes(DEFAULT_TRUNCATION_BYTES),
        supports_parallel_tool_calls: true,
        supports_image_detail_original: false,
        context_window: Some(m.max_context_length.unwrap_or(DEFAULT_CONTEXT_WINDOW)),
        auto_compact_token_limit: None,
        effective_context_window_percent: 95,
        experimental_supported_tools: Vec::new(),
        input_modalities: default_input_modalities(),
        used_fallback_model_metadata: false,
        supports_search_tool: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const FIXTURE: &str = include_str!("../tests/fixtures/mistral_models.json");

    #[test]
    fn filters_to_chat_capable_models() {
        let resp = translate_mistral_models(FIXTURE.as_bytes()).expect("translate");
        // Fixture: zai-glm-5-2 and mistral-medium-latest are chat; mistral-embed is not.
        assert_eq!(resp.models.len(), 2);
        assert!(resp.models.iter().all(|m| m.slug != "mistral-embed"));
    }

    #[test]
    fn mvp_model_has_expected_fields() {
        let resp = translate_mistral_models(FIXTURE.as_bytes()).expect("translate");
        let glm = resp
            .models
            .iter()
            .find(|m| m.slug == "zai-glm-5-2")
            .expect("glm present");
        assert_eq!(glm.display_name, "zai-glm-5-2");
        assert_eq!(glm.visibility, ModelVisibility::List);
        assert_eq!(glm.shell_type, ConfigShellToolType::ShellCommand);
        assert!(glm.supported_in_api);
        assert_eq!(glm.context_window, Some(131_072));
    }

    #[test]
    fn defaults_context_window_when_missing() {
        let resp = translate_mistral_models(FIXTURE.as_bytes()).expect("translate");
        let medium = resp
            .models
            .iter()
            .find(|m| m.slug == "mistral-medium-latest")
            .expect("medium present");
        assert_eq!(medium.context_window, Some(DEFAULT_CONTEXT_WINDOW));
    }

    #[test]
    fn output_round_trips_through_models_response_deserializer() {
        // Mirrors the exact call codex-api makes at endpoint/models.rs:64.
        let resp = translate_mistral_models(FIXTURE.as_bytes()).expect("translate");
        let json = serde_json::to_vec(&resp).expect("serialize");
        let reparsed: ModelsResponse = serde_json::from_slice(&json).expect("deserialize");
        assert_eq!(reparsed, resp);
    }

    #[test]
    fn empty_data_yields_empty_models() {
        let resp = translate_mistral_models(br#"{"object":"list","data":[]}"#).expect("translate");
        assert!(resp.models.is_empty());
    }
}
