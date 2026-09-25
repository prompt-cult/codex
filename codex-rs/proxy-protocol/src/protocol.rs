//! Prompt Cult Proxy Protocol v1 wire types.
//!
//! Normative specification: `docs/proxy-protocol.md`. These types cover the
//! v1 boot handshake (`server-info`) and the authenticated secret-push
//! supply channel. Conforming routers and proxies exchange these schemas;
//! any breaking wire change requires incrementing the protocol major
//! version.

use anyhow::Context;
use serde::Deserialize;
use serde::Serialize;
use zeroize::Zeroize;

/// The protocol major version these types implement.
pub const PROTOCOL_VERSION: u32 = 1;

/// Secret supply channels a proxy supports, per protocol v1 section 4.
///
/// Serialization is the lower-case hyphenated wire string; unknown channel
/// names arriving on the wire are minor/additive and are skipped so stale
/// routers keep working against newer proxies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretChannel {
    EnvDebug,
    SecretPush,
    WorkloadIdentity,
}

/// Skips unrecognized channel names on the wire (minor, additive).
impl<'de> Deserialize<'de> for SecretChannelList {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw: Vec<String> = Vec::deserialize(deserializer)?;
        Ok(SecretChannelList(
            raw.into_iter()
                .filter_map(|name| serde_json::from_value(serde_json::Value::String(name)).ok())
                .collect(),
        ))
    }
}

/// Capability advertisement: the secret supply channels this proxy
/// executable implements. A commercial proxy resolving its own secrets via
/// a platform identity ships `["workload-identity"]` and nothing else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretChannelList(pub Vec<SecretChannel>);

impl Serialize for SecretChannelList {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(Some(self.0.len()))?;
        for channel in &self.0 {
            seq.serialize_element(channel)?;
        }
        seq.end()
    }
}

/// v1 boot handshake written by a conforming proxy when started with
/// `--server-info <FILE>` (protocol v1 section 3). Single-line JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ServerInfoV1 {
    pub server_info_version: u32,
    pub port: u16,
    pub pid: u32,
    pub proxy_kind: String,
    /// Always [`PROTOCOL_VERSION`]; routers reject a greater major loudly.
    pub protocol_version: u32,
    pub secret_channels: SecretChannelList,
}

impl ServerInfoV1 {
    /// Writes the single-line descriptor to a file path, ending with a
    /// newline. The write is atomic: a sibling temp file is written first
    /// and renamed over the destination, so a poller never observes a
    /// partially written server-info file.
    pub fn write_to_file(&self, path: &std::path::Path) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.protocol_version == PROTOCOL_VERSION,
            "server info protocol_version {} does not match implemented {}",
            self.protocol_version,
            PROTOCOL_VERSION,
        );
        let mut data = serde_json::to_string(self)?;
        data.push('\n');
        let tmp_path = path.with_extension("tmp");
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp_path)?;
            f.write_all(data.as_bytes())?;
        }
        std::fs::rename(&tmp_path, path)?;
        Ok(())
    }
}

/// Deserialization with the v1 handshake validation rules: the file schema
/// version and protocol major must both be 1, and unknown channel names are
/// ignorable (minor, additive).
#[derive(Debug, Deserialize)]
struct ServerInfoV1Raw {
    server_info_version: u32,
    port: u16,
    pid: u32,
    proxy_kind: String,
    protocol_version: u32,
    #[serde(flatten)]
    _extra: serde_json::Map<String, serde_json::Value>,
}

impl TryFrom<ServerInfoV1Raw> for ServerInfoV1 {
    type Error = anyhow::Error;

    fn try_from(raw: ServerInfoV1Raw) -> anyhow::Result<Self> {
        anyhow::ensure!(
            raw.server_info_version == 1,
            "unknown server-info schema version {}",
            raw.server_info_version
        );
        anyhow::ensure!(
            raw.protocol_version == PROTOCOL_VERSION,
            "child advertised protocol_version {}; this router implements {}; {}",
            raw.protocol_version,
            PROTOCOL_VERSION,
            if raw.protocol_version > PROTOCOL_VERSION {
                "upgrade the router"
            } else {
                "unsupported legacy protocol version"
            },
        );
        Ok(ServerInfoV1 {
            server_info_version: raw.server_info_version,
            port: raw.port,
            pid: raw.pid,
            proxy_kind: raw.proxy_kind,
            protocol_version: raw.protocol_version,
            secret_channels: SecretChannelList(Vec::new()),
        })
    }
}

impl<'de> Deserialize<'de> for ServerInfoV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        let raw: serde_json::Value = serde_json::Value::deserialize(deserializer)?;
        // Parse secret_channels from the raw value: unknown names skipped.
        let secret_channels = match raw.get("secret_channels") {
            Some(list) => {
                let names: Vec<String> =
                    serde_json::from_value(list.clone()).map_err(D::Error::custom)?;
                SecretChannelList(
                    names
                        .into_iter()
                        .filter_map(|name| {
                            serde_json::from_value::<SecretChannel>(serde_json::Value::String(name))
                                .ok()
                        })
                        .collect(),
                )
            }
            None => SecretChannelList(Vec::new()),
        };

        let raw_struct: ServerInfoV1Raw = serde_json::from_value(raw).map_err(D::Error::custom)?;
        let mut info = ServerInfoV1::try_from(raw_struct).map_err(D::Error::custom)?;
        info.secret_channels = secret_channels;
        Ok(info)
    }
}

/// The authenticated direct memory push payload (protocol v1 section 4.2).
/// Authenticated by the ephemeral single-use boot token header; the proxy
/// ingests `material` into `mlock(2)` memory and zeroizes the request copy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretPushPayload {
    pub channel: SecretChannel,
    pub key_id: String,
    pub material: String,
}

/// Single-use boot token header name for the secret-push endpoint.
pub const BOOT_TOKEN_HEADER: &str = "X-Router-Boot-Token";

/// The authenticated secret-push endpoint path (protocol v1 section 4.2).
pub const SECRETS_ENDPOINT: &str = "/protocol/v1/secrets";

/// Thread-safe provisioning state for the key header (protocol v1 section
/// 4.2). In `secret-push` mode this starts empty and is filled by the
/// authenticated `POST /protocol/v1/secrets` delivery; in the other modes it
/// is populated at startup. Provisioning is atomic: exactly one of the
/// competing pushes wins, every loser must answer `409`. The boot token is
/// single-use: it is consumed the moment a secret is provisioned.
#[derive(Clone)]
pub struct SecretState {
    secret_push: bool,
    inner: std::sync::Arc<std::sync::RwLock<Option<&'static str>>>,
    boot_token: std::sync::Arc<std::sync::RwLock<Option<std::sync::Arc<String>>>>,
}

impl SecretState {
    /// State for a proxy that resolved its key at startup (`env-debug` or a
    /// commercial `workload-identity` implementation).
    pub fn provisioned_at_startup(auth_header: &'static str) -> Self {
        SecretState {
            secret_push: false,
            inner: std::sync::Arc::new(std::sync::RwLock::new(Some(auth_header))),
            boot_token: std::sync::Arc::new(std::sync::RwLock::new(None)),
        }
    }

    /// State for `secret-push` mode: starts keyless with an in-memory boot
    /// token and waits for the authenticated push.
    pub fn pending(boot_token: String) -> Self {
        SecretState {
            secret_push: true,
            inner: std::sync::Arc::new(std::sync::RwLock::new(None)),
            boot_token: std::sync::Arc::new(std::sync::RwLock::new(Some(std::sync::Arc::new(
                boot_token,
            )))),
        }
    }

    /// True when this proxy was started in `secret-push` mode.
    pub fn secret_push_mode(&self) -> bool {
        self.secret_push
    }

    /// The current key header, if provisioned.
    pub fn get(&self) -> Option<&'static str> {
        *self.inner.read().expect("secret state lock")
    }

    /// The boot token, while it is still valid (not yet consumed).
    pub fn boot_token(&self) -> Option<std::sync::Arc<String>> {
        self.boot_token.read().expect("boot token lock").clone()
    }

    /// Atomically provisions the key header. Returns `false` when a secret
    /// was already provisioned (replay); the caller must respond `409`.
    pub fn provision_once(&self, auth_header: &'static str) -> bool {
        let mut guard = self.inner.write().expect("secret state lock");
        if guard.is_some() {
            return false;
        }
        *guard = Some(auth_header);
        true
    }

    /// Invalidates the boot token (single use, protocol v1 section 4.2). The
    /// in-memory string is zeroized before it is dropped; this is
    /// best-effort when a concurrent reader still holds an `Arc` clone.
    pub fn consume_boot_token(&self) {
        if let Some(token) = self.boot_token.write().expect("boot token lock").take()
            && let Ok(mut owned) = std::sync::Arc::try_unwrap(token)
        {
            owned.zeroize();
        }
    }
}

/// Reads the ephemeral single-use boot token from its 0600 file (protocol v1
/// section 4.2). The file content is zeroized, overwritten, and the file
/// deleted immediately so the token exists only in memory and only until
/// provisioning consumes it. Scrub-before-validate ordering is intentional:
/// the secret is scrubbed from disk before its validity is checked.
pub fn read_boot_token_file(path: &std::path::Path) -> anyhow::Result<String> {
    let mut contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading boot token file {}", path.display()))?;
    let token = contents.trim().to_string();
    contents.zeroize();
    let zeros = vec![0u8; token.len()];
    let _ = std::fs::write(path, &zeros);
    std::fs::remove_file(path)
        .with_context(|| format!("removing boot token file {}", path.display()))?;
    anyhow::ensure!(!token.is_empty(), "boot token file must not be empty");
    Ok(token)
}

#[cfg(test)]
mod secret_state_tests {
    use super::SecretState;
    use pretty_assertions::assert_eq;

    #[test]
    fn pending_state_starts_keyless_with_a_live_token() {
        let state = SecretState::pending("token-1".to_string());
        assert!(state.secret_push_mode());
        assert_eq!(state.get(), None);
        assert_eq!(
            state.boot_token().as_deref().map(String::as_str),
            Some("token-1")
        );
    }

    #[test]
    fn provision_is_atomic_and_single_shot() {
        let state = SecretState::pending("token-1".to_string());
        assert!(state.provision_once("Bearer first"));
        assert!(!state.provision_once("Bearer second"));
        assert_eq!(state.get(), Some("Bearer first"));
    }

    #[test]
    fn boot_token_is_consumed_on_demand_and_once_only() {
        let state = SecretState::pending("token-1".to_string());
        state.consume_boot_token();
        assert_eq!(state.boot_token(), None);
        state.consume_boot_token();
        assert_eq!(state.boot_token(), None);
    }

    #[test]
    fn startup_provisioned_state_has_no_token_and_no_push_mode() {
        let state = SecretState::provisioned_at_startup("Bearer ready");
        assert!(!state.secret_push_mode());
        assert_eq!(state.get(), Some("Bearer ready"));
        assert_eq!(state.boot_token(), None);
    }
}
