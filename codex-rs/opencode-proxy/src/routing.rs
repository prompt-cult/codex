/// Model family classification for upstream routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelFamily {
    /// OpenAI Responses passthrough: gpt-*, o1*, o3*, grok-*, muse-*
    Gpt,
    /// Anthropic /messages translation: claude-*, minimax-*, qwen*
    Claude,
    /// OpenAI-compatible /chat/completions translation:
    /// glm-*, kimi-*, deepseek-*, longcat-*, mimo-*, hy-*
    Chat,
    Unknown,
}

/// Classify a model name into a family for routing decisions. Gpt is checked
/// first so that Chat's broad `hy` prefix can never swallow a gpt/o-family
/// model.
pub(crate) fn classify_model(model: &str) -> ModelFamily {
    if model.starts_with("gpt-")
        || model.starts_with("o1")
        || model.starts_with("o3")
        || model.starts_with("grok-")
        || model.starts_with("muse-")
    {
        ModelFamily::Gpt
    } else if model.starts_with("claude-")
        || model.starts_with("minimax-")
        || model.starts_with("qwen")
    {
        ModelFamily::Claude
    } else if model.starts_with("glm-")
        || model.starts_with("kimi-")
        || model.starts_with("deepseek-")
        || model.starts_with("longcat-")
        || model.starts_with("mimo-")
        || model.starts_with("hy-")
    {
        ModelFamily::Chat
    } else {
        ModelFamily::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn test_classify_gpt() {
        assert_eq!(classify_model("gpt-4o"), ModelFamily::Gpt);
        assert_eq!(classify_model("gpt-5.4"), ModelFamily::Gpt);
        assert_eq!(classify_model("gpt-5.4-mini"), ModelFamily::Gpt);
        assert_eq!(classify_model("o1-preview"), ModelFamily::Gpt);
        assert_eq!(classify_model("o3-mini"), ModelFamily::Gpt);
        assert_eq!(classify_model("grok-4"), ModelFamily::Gpt);
        assert_eq!(classify_model("muse-large"), ModelFamily::Gpt);
    }

    #[test]
    fn test_classify_claude() {
        assert_eq!(
            classify_model("claude-sonnet-4-20250514"),
            ModelFamily::Claude
        );
        assert_eq!(classify_model("claude-haiku-4-5"), ModelFamily::Claude);
        assert_eq!(classify_model("claude-3.5-sonnet"), ModelFamily::Claude);
        assert_eq!(classify_model("minimax-m2"), ModelFamily::Claude);
        assert_eq!(classify_model("qwen3-coder"), ModelFamily::Claude);
        assert_eq!(classify_model("qwen-coder"), ModelFamily::Claude);
    }

    #[test]
    fn test_classify_chat() {
        assert_eq!(classify_model("glm-5.3-flash"), ModelFamily::Chat);
        assert_eq!(classify_model("kimi-k2"), ModelFamily::Chat);
        assert_eq!(classify_model("deepseek-v4-flash"), ModelFamily::Chat);
        assert_eq!(classify_model("longcat-flash"), ModelFamily::Chat);
        assert_eq!(classify_model("mimo-7b"), ModelFamily::Chat);
        assert_eq!(classify_model("hy-unify"), ModelFamily::Chat);
    }

    #[test]
    fn test_classify_unknown() {
        assert_eq!(classify_model("gemini-pro"), ModelFamily::Unknown);
        assert_eq!(classify_model("llama-3"), ModelFamily::Unknown);
        assert_eq!(classify_model(""), ModelFamily::Unknown);
    }

    #[test]
    fn gpt_is_checked_before_chat_prefixes() {
        // `muse-` and `mimo-` share no characters but the ordering must keep
        // the Gpt branch first so no later prefix can shadow an o-family or
        // gpt-family name.
        assert_eq!(classify_model("gpt-hybrid"), ModelFamily::Gpt);
        assert_eq!(classify_model("o3-hy"), ModelFamily::Gpt);
        assert_eq!(classify_model("mimo-gpt-5"), ModelFamily::Chat);
    }
}
