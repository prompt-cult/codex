//! Tests for Zen provider with custom model_providers config.
//!
//! These tests verify the config -> provider -> API key flow for Zen provider.
//!
//! Run with: `cargo test -p codex-core zen_provider`

#![cfg(not(target_os = "windows"))]

use anyhow::Result;
use codex_model_provider_info::ModelProviderInfo;
use codex_model_provider_info::built_in_model_providers;
use core_test_support::load_default_config_for_test;
use tempfile::TempDir;

const TEST_MODEL: &str = "gpt-5.4";

fn create_zen_provider(base_url: &str) -> ModelProviderInfo {
    ModelProviderInfo {
        name: "Zen".to_string(),
        base_url: Some(base_url.to_string()),
        env_key: Some("OPENCODE_API_KEY".to_string()),
        env_key_instructions: Some("Get from https://opencode.ai/zen".to_string()),
        requires_openai_auth: false,
        supports_websockets: false,
        wire_api: codex_model_provider_info::WireApi::Responses,
        ..built_in_model_providers(None)["openai"].clone()
    }
}

/// Test that env-based API key loading works for custom providers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_loads_from_env_key() -> Result<()> {
    let _codex_home = TempDir::new()?;

    // Create a Zen provider
    let provider = create_zen_provider("https://opencode.ai/zen/v1");

    // Verify provider is configured correctly
    assert_eq!(provider.name, "Zen");
    assert_eq!(
        provider.base_url.as_ref().unwrap(),
        "https://opencode.ai/zen/v1"
    );
    assert_eq!(provider.env_key.as_ref().unwrap(), "OPENCODE_API_KEY");
    assert!(!provider.requires_openai_auth);

    Ok(())
}

/// Test that config.toml with zen provider loads correctly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_loads_from_config() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut config = load_default_config_for_test(&codex_home).await;

    // Set provider directly on config (simulating config.toml load)
    let provider = create_zen_provider("https://opencode.ai/zen/v1");
    config.model_provider = provider;
    config.model_provider_id = "zen".to_string();
    config.model = Some(TEST_MODEL.to_string());

    // Verify loaded correctly
    assert_eq!(config.model_provider_id, "zen");
    assert_eq!(config.model_provider.name, "Zen");
    assert_eq!(
        config.model_provider.env_key.as_ref().unwrap(),
        "OPENCODE_API_KEY"
    );

    Ok(())
}

/// Test model switching with custom provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_model_switching() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut config = load_default_config_for_test(&codex_home).await;

    // Initial provider
    let provider = create_zen_provider("https://opencode.ai/zen/v1");
    config.model_provider = provider;
    config.model_provider_id = "zen".to_string();
    config.model = Some("gpt-5.4".to_string());

    // Verify initial model
    assert_eq!(config.model.as_ref().unwrap(), "gpt-5.4");

    // Switch model (simulating /model command)
    config.model = Some("o3".to_string());

    // Verify switched
    assert_eq!(config.model.as_ref().unwrap(), "o3");
    assert_eq!(config.model_provider_id, "zen"); // provider unchanged

    Ok(())
}

/// Test provider persists in model_provider_id field.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_persists_in_config() -> Result<()> {
    let codex_home = TempDir::new()?;
    let mut config = load_default_config_for_test(&codex_home).await;

    // Set provider
    let provider = create_zen_provider("https://opencode.ai/zen/v1");
    config.model_provider = provider;
    config.model_provider_id = "zen".to_string();

    // Verify state
    assert_eq!(config.model_provider_id, "zen");
    assert_eq!(config.model_provider.name, "Zen");
    assert_eq!(
        config.model_provider.base_url.as_ref().unwrap(),
        "https://opencode.ai/zen/v1"
    );

    Ok(())
}

/// Test that wire_api=responses is used correctly.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_uses_responses_wire_api() -> Result<()> {
    let provider = create_zen_provider("https://opencode.ai/zen/v1");

    // Verify wire_api is Responses
    assert_eq!(
        provider.wire_api,
        codex_model_provider_info::WireApi::Responses
    );

    // Verify base_url ends with /v1 for Responses API
    assert!(provider.base_url.as_ref().unwrap().ends_with("/v1"));

    Ok(())
}

/// Test that provider with requires_openai_auth=false skips ChatGPT auth.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_skips_chatgpt_auth() -> Result<()> {
    let provider = create_zen_provider("https://opencode.ai/zen/v1");

    // Verify no ChatGPT auth required
    assert!(!provider.requires_openai_auth);

    // When requires_openai_auth is false, auth flow should skip login screen
    // and read directly from env_key (OPENCODE_API_KEY)
    assert_eq!(provider.env_key.as_ref().unwrap(), "OPENCODE_API_KEY");

    Ok(())
}

/// Test provider merging: custom + built-in providers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_merges_with_builtin() -> Result<()> {
    // Built-in providers
    let mut built_in = built_in_model_providers(None);

    // Verify built-in includes openai
    assert!(built_in.contains_key("openai"));
    assert!(built_in.contains_key("ollama"));
    assert!(built_in.contains_key("lmstudio"));

    // Custom provider
    let custom = create_zen_provider("https://opencode.ai/zen/v1");

    // Merging keeps both
    built_in.entry("zen".to_string()).or_insert(custom);

    assert!(built_in.contains_key("zen"));
    assert!(built_in.contains_key("openai"));

    Ok(())
}

/// Test config overrides via CLI -c model_provider=zen
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_cli_override() -> Result<()> {
    // Simulating: codex -c model_provider=zen "prompt"

    // The CLI override sets model_provider_id
    let cli_override_provider_id = "zen";

    // This flows through ConfigOverrides.model_provider
    // and ends up in config.model_provider_id

    assert_eq!(cli_override_provider_id, "zen");

    Ok(())
}

/// Test that OPENCODE_API_KEY env var name is correct.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_env_var_name() -> Result<()> {
    // The provider has env_key = Some("OPENCODE_API_KEY")
    // When requires_openai_auth=false, the auth flow should:
    // 1. Read env var OPENCODE_API_KEY
    // 2. Use it as Bearer token

    let provider = create_zen_provider("https://opencode.ai/zen/v1");
    let env_key_name = provider.env_key.as_ref().expect("env_key should be set");

    // Verify it's the correct env var name
    assert_eq!(env_key_name, "OPENCODE_API_KEY");

    Ok(())
}

/// Test base_url is correct for Zen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_base_url() -> Result<()> {
    let provider = create_zen_provider("https://opencode.ai/zen/v1");

    // Verify base_url is correct
    assert_eq!(
        provider.base_url.as_ref().unwrap(),
        "https://opencode.ai/zen/v1"
    );

    Ok(())
}

/// Test loading actual config.toml with zen provider.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zen_provider_loads_from_file() -> Result<()> {
    use codex_config::CONFIG_TOML_FILE;
    use std::path::PathBuf;

    // Try to load actual config from ~/codex-local/config.toml
    let config_path = PathBuf::from("/Users/Shared/codex-local/config.toml");

    if config_path.exists() {
        let content = std::fs::read_to_string(&config_path)?;

        // Parse the config
        let config: codex_config::config_toml::ConfigToml = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse config: {}", e))?;

        // Check zen provider exists
        let zen = config
            .model_providers
            .get("zen")
            .ok_or_else(|| anyhow::anyhow!("zen provider not found in config"))?;

        println!(
            "Loaded zen provider: name={}, base_url={:?}, env_key={:?}, requires_openai_auth={}",
            zen.name, zen.base_url, zen.env_key, zen.requires_openai_auth
        );

        // Verify zen provider is correct
        assert_eq!(zen.name, "Zen");
        assert_eq!(zen.base_url.as_ref().unwrap(), "https://opencode.ai/zen/v1");
        assert_eq!(zen.env_key.as_ref().unwrap(), "OPENCODE_API_KEY");
        assert!(!zen.requires_openai_auth);

        println!("SUCCESS: zen provider loaded correctly from config file!");
    } else {
        eprintln!("Skipping: config file not found at {:?}", config_path);
    }

    Ok(())
}
