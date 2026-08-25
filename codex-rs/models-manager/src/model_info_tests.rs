use super::*;
use crate::ModelsManagerConfig;
use pretty_assertions::assert_eq;

#[test]
fn reasoning_summaries_override_true_enables_support() {
    let model = model_info_from_slug("unknown-model");
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(true),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);
    let mut expected = model;
    expected.supports_reasoning_summaries = true;

    assert_eq!(updated, expected);
}

#[test]
fn reasoning_summaries_override_false_does_not_disable_support() {
    let mut model = model_info_from_slug("unknown-model");
    model.supports_reasoning_summaries = true;
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(false),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn reasoning_summaries_override_false_is_noop_when_model_is_false() {
    let model = model_info_from_slug("unknown-model");
    let config = ModelsManagerConfig {
        model_supports_reasoning_summaries: Some(false),
        ..Default::default()
    };

    let updated = with_config_overrides(model.clone(), &config);

    assert_eq!(updated, model);
}

#[test]
fn mistral_hosted_models_get_128k_context_window_via_local_proxy_defaults() {
    // The provider-agnostic fallback always reports the default window…
    let glm_fallback = model_info_from_slug("zai-glm-5-2");
    assert_eq!(glm_fallback.context_window, Some(272_000));

    // …and the proxy-aware adjustment applies Mistral-hosted limits only when
    // the provider is known to be a local proxy (applied at the call site).
    for slug in [
        "zai-glm-5-2",
        "mistral-medium-latest",
        "mistral-large-latest",
        "devstral",
        "codestral-latest",
    ] {
        let adjusted = with_local_proxy_defaults(model_info_from_slug(slug), slug);
        assert_eq!(adjusted.context_window, Some(128_000), "slug: {slug}");
    }
}

#[test]
fn local_proxy_defaults_leave_other_slugs_untouched() {
    // A GPT model served by a different local proxy (e.g. zen-proxy) must not
    // inherit Mistral limits.
    let adjusted = with_local_proxy_defaults(model_info_from_slug("gpt-5.4"), "gpt-5.4");
    assert_eq!(adjusted.context_window, Some(272_000));
}

#[test]
fn non_mistral_models_keep_default_context_window() {
    let unknown = model_info_from_slug("unknown-model");
    assert_eq!(unknown.context_window, Some(272_000));
}
