//! Compiled-in set of backend proxies the routing dispatcher routes to.
//!
//! Each entry pairs the public path prefix (the upstream host name) with the
//! sibling executable booted as a child process and the allow-list of
//! environment variable names the child may inherit. Adding a future proxy
//! is one entry in [`BACKENDS`]; see `README.proxy-router.md` for the
//! dispatcher contract.

/// Registry entry describing one backend proxy.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) struct Backend {
    /// Path prefix that selects this backend, e.g. "/opencode.ai".
    pub prefix: &'static str,
    /// Sibling executable name booted next to the dispatcher binary.
    pub binary: &'static str,
    /// Upstream host label used for logs and health output.
    pub label: &'static str,
    /// Environment variable names the child is allowed to inherit. Names on
    /// this list that exist (non-empty) in the dispatcher's environment are
    /// copied opaquely into the child's sanitized environment; every other
    /// name is dropped. Values are never inspected, logged, or stored.
    ///
    /// Transitional affordance: the end state is that children resolve their
    /// secrets themselves through workload identities (a KMS, a Kubernetes
    /// volume mount, or another hardened key store) and nothing is passed.
    pub allowed_env: &'static [&'static str],
}

/// The compiled-in backend set.
pub(crate) const BACKENDS: &[Backend] = &[
    Backend {
        prefix: "/opencode.ai",
        binary: "codex-opencode-proxy",
        label: "opencode.ai",
        allowed_env: &["OPENCODE_API_KEY"],
    },
    Backend {
        prefix: "/mistral.ai",
        binary: "codex-mistral-proxy",
        label: "mistral.ai",
        allowed_env: &["MISTRAL_API_KEY"],
    },
];

#[cfg(test)]
mod tests {
    use super::BACKENDS;
    use pretty_assertions::assert_eq;

    #[test]
    fn adding_a_future_proxy_is_one_entry() {
        assert_eq!(BACKENDS.len(), 2);
        assert_eq!(BACKENDS[0].prefix, "/opencode.ai");
        assert_eq!(BACKENDS[0].binary, "codex-opencode-proxy");
        assert_eq!(BACKENDS[1].prefix, "/mistral.ai");
        assert_eq!(BACKENDS[1].binary, "codex-mistral-proxy");
    }

    #[test]
    fn allow_lists_name_exactly_the_one_key_vault() {
        assert_eq!(BACKENDS[0].allowed_env, &["OPENCODE_API_KEY"]);
        assert_eq!(BACKENDS[1].allowed_env, &["MISTRAL_API_KEY"]);
    }
}
