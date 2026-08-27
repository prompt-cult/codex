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

/// Result of translating a Mistral `/models` payload.
#[derive(Debug)]
pub struct TranslatedModels {
    /// The codex `ModelsResponse` to serve the client.
    pub response: ModelsResponse,
    /// Chat-capable models seen upstream before glob exclusions, so the proxy
    /// can log `loaded N models, M after exclusions`.
    pub chat_loaded: usize,
}

/// Per-model metadata override applied during translation.
#[derive(Debug, Default, Clone)]
pub struct ModelTranslateOverride {
    /// Replacement system instructions for the discovered model.
    pub base_instructions: Option<String>,
}

/// Parse raw Mistral `/models` JSON and build a codex [`ModelsResponse`].
///
/// Models whose ID matches any glob in `exclude` are dropped before
/// priorities are assigned, so the surviving list is densely ordered.
/// `overrides` is keyed by upstream model ID.
pub fn translate_mistral_models(
    raw: &[u8],
    exclude: &globset::GlobSet,
    overrides: &std::collections::HashMap<String, ModelTranslateOverride>,
) -> Result<TranslatedModels> {
    let list: MistralModelList =
        serde_json::from_slice(raw).context("parsing Mistral /models response")?;

    let chat_capable: Vec<MistralModel> = list
        .data
        .into_iter()
        .filter(|m| m.capabilities.completion_chat)
        .collect();
    let chat_loaded = chat_capable.len();

    let models = chat_capable
        .into_iter()
        .filter(|m| !exclude.is_match(&m.id))
        .enumerate()
        .map(|(index, m)| {
            let ovr = overrides.get(&m.id).cloned().unwrap_or_default();
            model_info_for(index, m, ovr)
        })
        .collect();

    Ok(TranslatedModels {
        response: ModelsResponse { models },
        chat_loaded,
    })
}

/// Build a fully-populated [`ModelInfo`] for a chat-capable Mistral model.
///
/// `ModelInfo` has no `Default`, so every field is set explicitly. Optional and
/// reasoning-related metadata Mistral does not provide is left empty/`None`;
/// users can override context window and related limits via config.
/// Codex base instructions served for every discovered model unless a
/// `model_overrides` entry replaces them for a specific model. Includes an
/// identity line so models answer "what model are you?" with the selected
/// model ID instead of an upstream alias name.
const DEFAULT_BASE_INSTRUCTIONS: &str = include_str!("../prompt.md");

fn model_info_for(index: usize, m: MistralModel, ovr: ModelTranslateOverride) -> ModelInfo {
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
        base_instructions: ovr
            .base_instructions
            .unwrap_or_else(|| DEFAULT_BASE_INSTRUCTIONS.to_string()),
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
    use globset::Glob;
    use globset::GlobSetBuilder;
    use pretty_assertions::assert_eq;

    const FIXTURE: &str = include_str!("../tests/fixtures/mistral_models.json");

    fn no_exclusions() -> globset::GlobSet {
        GlobSetBuilder::new().build().expect("empty globset")
    }

    fn no_overrides() -> std::collections::HashMap<String, ModelTranslateOverride> {
        std::collections::HashMap::new()
    }

    fn globset_of(patterns: &[&str]) -> globset::GlobSet {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(Glob::new(pattern).expect("valid glob"));
        }
        builder.build().expect("globset")
    }

    #[test]
    fn filters_to_chat_capable_models() {
        let out = translate_mistral_models(FIXTURE.as_bytes(), &no_exclusions(), &no_overrides())
            .expect("translate");
        // Fixture: zai-glm-5-2 and mistral-medium-latest are chat; mistral-embed is not.
        assert_eq!(out.response.models.len(), 2);
        assert_eq!(out.chat_loaded, 2);
        assert!(
            out.response
                .models
                .iter()
                .all(|m| m.slug != "mistral-embed")
        );
    }

    #[test]
    fn exclusion_globs_drop_matching_models() {
        let exclude = globset_of(&["zai-glm-5-2", "*-medium-*"]);
        let out = translate_mistral_models(FIXTURE.as_bytes(), &exclude, &no_overrides())
            .expect("translate");
        assert!(out.response.models.is_empty());
        assert_eq!(out.chat_loaded, 2);
    }

    #[test]
    fn exact_exclusion_keeps_prefixed_variant() {
        let exclude = globset_of(&["glm-5-2"]);
        let raw = br#"{"object":"list","data":[
            {"id":"glm-5-2","capabilities":{"completion_chat":true}},
            {"id":"zai-glm-5-2","capabilities":{"completion_chat":true}}
        ]}"#;
        let out = translate_mistral_models(raw, &exclude, &no_overrides()).expect("translate");
        assert_eq!(out.response.models.len(), 1);
        assert_eq!(out.response.models[0].slug, "zai-glm-5-2");
        // Priorities are dense after filtering.
        assert_eq!(out.response.models[0].priority, 0);
    }

    #[test]
    fn mvp_model_has_expected_fields() {
        let out = translate_mistral_models(FIXTURE.as_bytes(), &no_exclusions(), &no_overrides())
            .expect("translate");
        let glm = out
            .response
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
        let out = translate_mistral_models(FIXTURE.as_bytes(), &no_exclusions(), &no_overrides())
            .expect("translate");
        let medium = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "mistral-medium-latest")
            .expect("medium present");
        assert_eq!(medium.context_window, Some(DEFAULT_CONTEXT_WINDOW));
    }

    #[test]
    fn output_round_trips_through_models_response_deserializer() {
        // Mirrors the exact call codex-api makes at endpoint/models.rs:64.
        let out = translate_mistral_models(FIXTURE.as_bytes(), &no_exclusions(), &no_overrides())
            .expect("translate");
        let json = serde_json::to_vec(&out.response).expect("serialize");
        let reparsed: ModelsResponse = serde_json::from_slice(&json).expect("deserialize");
        assert_eq!(reparsed, out.response);
    }

    #[test]
    fn default_instructions_come_from_prompt_md() {
        let out = translate_mistral_models(FIXTURE.as_bytes(), &no_exclusions(), &no_overrides())
            .expect("translate");
        let glm = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "zai-glm-5-2")
            .expect("glm present");
        assert!(glm.base_instructions.contains("Prompt Cult"));
        assert!(glm.base_instructions.contains("Model identity"));
    }

    #[test]
    fn override_replaces_base_instructions_for_one_model() {
        let mut overrides = std::collections::HashMap::new();
        overrides.insert(
            "zai-glm-5-2".to_string(),
            ModelTranslateOverride {
                base_instructions: Some("You are zai-glm-5-2, period.".to_string()),
            },
        );
        let out = translate_mistral_models(FIXTURE.as_bytes(), &no_exclusions(), &overrides)
            .expect("translate");
        let glm = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "zai-glm-5-2")
            .expect("glm present");
        assert_eq!(glm.base_instructions, "You are zai-glm-5-2, period.");
        let medium = out
            .response
            .models
            .iter()
            .find(|m| m.slug == "mistral-medium-latest")
            .expect("medium present");
        assert!(medium.base_instructions.contains("Prompt Cult"));
    }

    #[test]
    fn empty_data_yields_empty_models() {
        let out = translate_mistral_models(
            br#"{"object":"list","data":[]}"#,
            &no_exclusions(),
            &no_overrides(),
        )
        .expect("translate");
        assert!(out.response.models.is_empty());
        assert_eq!(out.chat_loaded, 0);
    }
}
