//! Shared contract for Prompt Cult provider proxies.
//!
//! See `docs/proxy-protocol.md`. Every proxy has a unique [`ProxyKind`]
//! identifier, loads an optional per-proxy JSONC settings file from the codex
//! config directory (`<proxy-id>.jsonc`), and filters discovered models
//! through glob exclusions.

use std::path::Path;

use anyhow::Context;
use anyhow::Result;
use globset::Glob;
use globset::GlobSet;
use globset::GlobSetBuilder;
use serde::Deserialize;

/// Unique, stable identifier for each provider proxy. The string form is the
/// config filename stem and the log-line prefix; it must never change once a
/// proxy ships, or existing user configs silently stop loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    MistralAi,
    OpencodeZen,
    OpencodeGo,
}

impl ProxyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProxyKind::MistralAi => "proxy-mistral-ai",
            ProxyKind::OpencodeZen => "proxy-opencode-zen",
            ProxyKind::OpencodeGo => "proxy-opencode-go",
        }
    }

    /// Filename of this proxy's settings file inside the codex config dir.
    pub fn config_filename(self) -> String {
        format!("{}.jsonc", self.as_str())
    }
}

/// Proxy logging verbosity, set via the `log_level` settings key.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[default]
    Normal,
    Verbose,
}

/// Compiled-in defaults a proxy supplies when no settings file exists. Also
/// the fallback for individual keys the file omits.
#[derive(Debug, Clone)]
pub struct ProxyDefaults {
    pub upstream_base_url: &'static str,
    pub model_exclude_globs: &'static [&'static str],
}

/// Raw shape of `<proxy-id>.jsonc`. Every key is optional; omitted keys fall
/// back to the proxy's [`ProxyDefaults`].
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct SettingsFile {
    upstream_base_url: Option<String>,
    model_exclude_globs: Option<Vec<String>>,
    log_level: Option<LogLevel>,
    /// Per-model metadata overrides applied to discovered models. The key is
    /// the upstream model ID; unknown keys are ignored so stale entries do
    /// not break discovery when a provider retires a model.
    model_overrides: Option<std::collections::HashMap<String, ModelOverride>>,
}

/// Metadata override for one discovered model. Every field is optional; a
/// present field replaces the value the proxy would otherwise synthesize.
#[derive(Debug, Default, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ModelOverride {
    /// System instructions served with the model in discovery. Codex uses
    /// these verbatim as the session's base instructions, which is the
    /// correct place to pin identity for models that self-report a
    /// canonicalized name (e.g. a `zai-glm-*` model answering
    /// "I am mistral-code-agent-latest").
    pub base_instructions: Option<String>,
    /// Absolute path to a UTF-8 file whose contents replace
    /// `base_instructions`, for instructions too large for inline JSONC.
    pub base_instructions_file: Option<std::path::PathBuf>,
}

/// The effective configuration a proxy runs with.
#[derive(Debug)]
pub struct ResolvedConfig {
    /// Upstream override from the file, if any. CLI flags take precedence;
    /// the caller falls back to [`ProxyDefaults::upstream_base_url`].
    pub upstream_base_url: Option<String>,
    /// Compiled exclusion set; matches are dropped from model discovery.
    pub exclude: GlobSet,
    /// Number of glob patterns in [`Self::exclude`], for startup logging.
    pub exclude_pattern_count: usize,
    pub log_level: LogLevel,
    /// Resolved per-model overrides (`base_instructions_file` contents
    /// inlined), keyed by upstream model ID.
    pub model_overrides: std::collections::HashMap<String, ModelOverride>,
    /// Absolute path of the settings file when one was loaded; `None` means
    /// compiled-in defaults are in use.
    pub loaded_from: Option<std::path::PathBuf>,
}

/// Load and resolve `<config_dir>/<proxy-id>.jsonc` against `defaults`.
///
/// A missing file resolves entirely to `defaults`. A present but malformed
/// file (bad JSONC, unknown key, invalid glob) is a hard error — silently
/// ignoring a broken config is how the wrong model ends up running.
pub fn load_config(
    kind: ProxyKind,
    config_dir: &Path,
    defaults: &ProxyDefaults,
) -> Result<ResolvedConfig> {
    let path = config_dir.join(kind.config_filename());
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return resolve(None, defaults, None);
        }
        Err(err) => {
            return Err(err).with_context(|| format!("reading proxy config {}", path.display()));
        }
    };
    let file: SettingsFile = json5::from_str(&raw)
        .with_context(|| format!("parsing proxy config {}", path.display()))?;
    resolve(Some(file), defaults, Some(path))
}

fn resolve(
    file: Option<SettingsFile>,
    defaults: &ProxyDefaults,
    loaded_from: Option<std::path::PathBuf>,
) -> Result<ResolvedConfig> {
    let file = file.unwrap_or_default();

    let patterns: Vec<String> = match file.model_exclude_globs {
        Some(list) => list,
        None => defaults
            .model_exclude_globs
            .iter()
            .map(ToString::to_string)
            .collect(),
    };

    let mut builder = GlobSetBuilder::new();
    for pattern in &patterns {
        let glob = Glob::new(pattern)
            .with_context(|| format!("invalid model exclusion glob {pattern:?}"))?;
        builder.add(glob);
    }
    let exclude = builder.build().context("compiling model exclusion globs")?;

    let mut model_overrides = std::collections::HashMap::new();
    for (model_id, ovr) in file.model_overrides.unwrap_or_default() {
        if ovr.base_instructions.is_some() && ovr.base_instructions_file.is_some() {
            anyhow::bail!(
                "model override for {model_id:?} sets both base_instructions and \
                 base_instructions_file; pick one"
            );
        }
        let ovr = if let Some(path) = ovr.base_instructions_file.clone() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading base_instructions_file {}", path.display()))?;
            ModelOverride {
                base_instructions: Some(text),
                base_instructions_file: None,
            }
        } else {
            ovr
        };
        model_overrides.insert(model_id, ovr);
    }

    Ok(ResolvedConfig {
        upstream_base_url: file.upstream_base_url,
        exclude_pattern_count: patterns.len(),
        exclude,
        log_level: file.log_level.unwrap_or_default(),
        model_overrides,
        loaded_from,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    const MISTRAL_DEFAULTS: ProxyDefaults = ProxyDefaults {
        upstream_base_url: "https://api.mistral.ai/v1",
        model_exclude_globs: &[
            "*-ocr-*",
            "*-mini-*",
            "magistral-*",
            "ministral-*",
            "voxtral-*",
            "glm-5-2",
        ],
    };

    fn load(dir: &Path) -> Result<ResolvedConfig> {
        load_config(ProxyKind::MistralAi, dir, &MISTRAL_DEFAULTS)
    }

    #[test]
    fn proxy_ids_are_stable() {
        assert_eq!(ProxyKind::MistralAi.as_str(), "proxy-mistral-ai");
        assert_eq!(ProxyKind::OpencodeZen.as_str(), "proxy-opencode-zen");
        assert_eq!(ProxyKind::OpencodeGo.as_str(), "proxy-opencode-go");
        assert_eq!(
            ProxyKind::MistralAi.config_filename(),
            "proxy-mistral-ai.jsonc"
        );
    }

    #[test]
    fn missing_file_uses_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = load(dir.path()).expect("load");
        assert_eq!(cfg.loaded_from, None);
        assert_eq!(cfg.upstream_base_url, None);
        assert_eq!(cfg.log_level, LogLevel::Normal);
        assert_eq!(cfg.exclude_pattern_count, 6);
    }

    #[test]
    fn default_globs_cover_prefix_suffix_contains_and_exact() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cfg = load(dir.path()).expect("load");
        let excluded = |id: &str| cfg.exclude.is_match(id);

        assert!(excluded("mistral-ocr-2512"));
        assert!(excluded("mistral-ocr-latest"));
        assert!(excluded("voxtral-mini-latest"));
        assert!(excluded("voxtral-mini-tts-2603"));
        assert!(excluded("voxtral-small-latest"));
        assert!(excluded("magistral-small-latest"));
        assert!(excluded("ministral-3b-2512"));
        assert!(excluded("ministral-14b-latest"));
        // Exact-match ban of the bare alias only.
        assert!(excluded("glm-5-2"));
        assert!(!excluded("zai-glm-5-2"));

        assert!(!excluded("mistral-medium-latest"));
        assert!(!excluded("mistral-large-latest"));
        assert!(!excluded("mistral-code-agent-latest"));
        assert!(!excluded("devstral-latest"));
        assert!(!excluded("codestral-latest"));
    }

    #[test]
    fn file_overrides_globs_and_endpoint_and_log_level() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            r#"{
                // jsonc comments must parse
                "upstream_base_url": "https://example.invalid/v1",
                "model_exclude_globs": ["devstral-*",],
                "log_level": "verbose",
            }"#,
        )
        .expect("write config");
        let cfg = load(dir.path()).expect("load");
        assert_eq!(
            cfg.loaded_from,
            Some(dir.path().join("proxy-mistral-ai.jsonc"))
        );
        assert_eq!(
            cfg.upstream_base_url.as_deref(),
            Some("https://example.invalid/v1")
        );
        assert_eq!(cfg.log_level, LogLevel::Verbose);
        assert_eq!(cfg.exclude_pattern_count, 1);
        assert!(cfg.exclude.is_match("devstral-latest"));
        // Overriding the list replaces the defaults entirely.
        assert!(!cfg.exclude.is_match("ministral-3b-2512"));
    }

    #[test]
    fn empty_glob_list_disables_filtering() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            r#"{ "model_exclude_globs": [] }"#,
        )
        .expect("write config");
        let cfg = load(dir.path()).expect("load");
        assert_eq!(cfg.exclude_pattern_count, 0);
        assert!(!cfg.exclude.is_match("mistral-ocr-2512"));
    }

    #[test]
    fn model_overrides_inline_instructions() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            r#"{
                "model_overrides": {
                    "zai-glm-5-2": {
                        "base_instructions": "You are zai-glm-5-2."
                    }
                }
            }"#,
        )
        .expect("write config");
        let cfg = load(dir.path()).expect("load");
        let ovr = cfg
            .model_overrides
            .get("zai-glm-5-2")
            .expect("override present");
        assert_eq!(
            ovr.base_instructions.as_deref(),
            Some("You are zai-glm-5-2.")
        );
    }

    #[test]
    fn model_overrides_instructions_file_is_inlined() {
        let dir = tempfile::tempdir().expect("tempdir");
        let instructions_path = dir.path().join("glm-instructions.md");
        std::fs::write(&instructions_path, "You are zai-glm-5-2 from a file.").expect("write md");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            format!(
                r#"{{ "model_overrides": {{ "zai-glm-5-2": {{ "base_instructions_file": "{}" }} }} }}"#,
                instructions_path.display()
            ),
        )
        .expect("write config");
        let cfg = load(dir.path()).expect("load");
        let ovr = cfg
            .model_overrides
            .get("zai-glm-5-2")
            .expect("override present");
        assert_eq!(
            ovr.base_instructions.as_deref(),
            Some("You are zai-glm-5-2 from a file.")
        );
    }

    #[test]
    fn model_overrides_both_fields_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            r#"{
                "model_overrides": {
                    "x": {
                        "base_instructions": "a",
                        "base_instructions_file": "/tmp/b.md"
                    }
                }
            }"#,
        )
        .expect("write config");
        load(dir.path()).expect_err("conflicting fields must fail");
    }

    #[test]
    fn model_overrides_missing_file_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            r#"{
                "model_overrides": {
                    "x": { "base_instructions_file": "/nonexistent/nope.md" }
                }
            }"#,
        )
        .expect("write config");
        let err = load(dir.path()).expect_err("missing file must fail");
        assert!(format!("{err:#}").contains("nope.md"));
    }

    #[test]
    fn malformed_jsonc_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            "{ this is not jsonc",
        )
        .expect("write config");
        let err = load(dir.path()).expect_err("must fail");
        assert!(format!("{err:#}").contains("proxy-mistral-ai.jsonc"));
    }

    #[test]
    fn unknown_key_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            r#"{ "modle_exclude_globs": ["x"] }"#,
        )
        .expect("write config");
        load(dir.path()).expect_err("typo key must fail");
    }

    #[test]
    fn invalid_glob_is_a_hard_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("proxy-mistral-ai.jsonc"),
            r#"{ "model_exclude_globs": ["[unclosed"] }"#,
        )
        .expect("write config");
        let err = load(dir.path()).expect_err("must fail");
        assert!(format!("{err:#}").contains("[unclosed"));
    }
}
