use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const SCOPE_READ: &str = "read";
pub const SCOPE_INFER: &str = "infer";
pub const SCOPE_CONTROL: &str = "control";
pub const SCOPE_AUDIT: &str = "audit";

const ALLOWED_SCOPES: &[&str] = &["*", SCOPE_READ, SCOPE_INFER, SCOPE_CONTROL, SCOPE_AUDIT];
const MINIMUM_KEY_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyRole {
    #[default]
    Management,
    Api,
}

impl KeyRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Management => "management",
            Self::Api => "api",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiPrincipal {
    pub name: String,
    pub role: KeyRole,
    pub scopes: BTreeSet<String>,
    pub fingerprint: String,
}

impl ApiPrincipal {
    pub fn allows(&self, required_scope: &str) -> bool {
        self.scopes.contains("*") || self.scopes.contains(required_scope)
    }

    pub fn allows_path(&self, path: &str) -> bool {
        match self.role {
            KeyRole::Management => true,
            KeyRole::Api => path == "/api/v1/auth/whoami" || path.contains("/jobs/"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AuthRegistry {
    principals: Arc<HashMap<[u8; 32], ApiPrincipal>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    keys: Vec<KeyDefinition>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyDefinition {
    name: String,
    key: String,
    scopes: Vec<String>,
    #[serde(default)]
    role: KeyRole,
    #[serde(default = "enabled_default")]
    enabled: bool,
}

fn enabled_default() -> bool {
    true
}

impl AuthRegistry {
    pub fn from_env() -> Result<Self> {
        let path = std::env::var_os("RKSERVE_API_KEYS_FILE")
            .map(PathBuf::from)
            .context("RKSERVE_API_KEYS_FILE is required; RKServe refuses to start without API authentication")?;
        Self::load(&path)
    }

    pub fn load(path: &Path) -> Result<Self> {
        validate_key_file(path)?;
        let raw = fs::read(path)
            .with_context(|| format!("read API key file {}", path.display()))?;
        let definitions: KeyFile = serde_json::from_slice(&raw)
            .with_context(|| format!("parse API key file {}", path.display()))?;
        anyhow::ensure!(!definitions.keys.is_empty(), "API key file must contain at least one key");

        let mut names = HashSet::new();
        let mut secrets = HashSet::new();
        let mut principals = HashMap::new();
        for definition in definitions.keys {
            let name = definition.name.trim();
            anyhow::ensure!(
                !name.is_empty()
                    && name.len() <= 64
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')),
                "API key name must be 1-64 ASCII letters, digits, '.', '_' or '-'"
            );
            anyhow::ensure!(names.insert(name.to_owned()), "duplicate API key name '{name}'");
            anyhow::ensure!(
                definition.key.as_bytes().len() >= MINIMUM_KEY_BYTES,
                "API key '{name}' must contain at least {MINIMUM_KEY_BYTES} bytes"
            );
            anyhow::ensure!(
                definition
                    .key
                    .bytes()
                    .all(|byte| (0x21..=0x7e).contains(&byte)),
                "API key '{name}' must contain only printable ASCII without whitespace"
            );
            let lowercase = definition.key.to_ascii_lowercase();
            anyhow::ensure!(
                !["change-me", "changeme", "placeholder", "replace-me", "example"]
                    .iter()
                    .any(|marker| lowercase.contains(marker)),
                "API key '{name}' looks like a placeholder"
            );
            anyhow::ensure!(!definition.scopes.is_empty(), "API key '{name}' has no scopes");
            let scopes = definition
                .scopes
                .into_iter()
                .map(|scope| {
                    anyhow::ensure!(
                        ALLOWED_SCOPES.contains(&scope.as_str()),
                        "API key '{name}' has unknown scope '{scope}'"
                    );
                    Ok(scope)
                })
                .collect::<Result<BTreeSet<_>>>()?;
            match definition.role {
                KeyRole::Api => anyhow::ensure!(
                    scopes.len() == 1 && scopes.contains(SCOPE_INFER),
                    "API key '{name}' with role=api can only grant the infer scope"
                ),
                KeyRole::Management => anyhow::ensure!(
                    scopes.contains("*")
                        || scopes.contains(SCOPE_READ)
                        || scopes.contains(SCOPE_CONTROL)
                        || scopes.contains(SCOPE_AUDIT),
                    "API key '{name}' with role=management needs read, control, audit or *"
                ),
            }
            let digest = Sha256::digest(definition.key.as_bytes());
            let digest: [u8; 32] = digest.into();
            anyhow::ensure!(secrets.insert(digest), "the same API key is assigned more than once");
            if !definition.enabled {
                continue;
            }
            principals.insert(
                digest,
                ApiPrincipal {
                    name: name.to_owned(),
                    role: definition.role,
                    scopes,
                    fingerprint: fingerprint(&digest),
                },
            );
        }
        anyhow::ensure!(
            !principals.is_empty(),
            "API key file must contain at least one enabled key"
        );

        Ok(Self {
            principals: Arc::new(principals),
        })
    }

    pub fn authenticate(&self, key: &str) -> Option<ApiPrincipal> {
        let digest: [u8; 32] = Sha256::digest(key.as_bytes()).into();
        self.principals.get(&digest).cloned()
    }

    pub fn key_count(&self) -> usize {
        self.principals.len()
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrustedProxies {
    addresses: Arc<HashSet<IpAddr>>,
}

impl TrustedProxies {
    pub fn from_env() -> Result<Self> {
        let Some(raw) = std::env::var_os("RKSERVE_TRUSTED_PROXIES") else {
            return Ok(Self::default());
        };
        let raw = raw.to_string_lossy();
        let mut addresses = HashSet::new();
        for value in raw.split(',').map(str::trim).filter(|value| !value.is_empty()) {
            let address = value
                .parse::<IpAddr>()
                .with_context(|| format!("parse trusted proxy IP '{value}'"))?;
            addresses.insert(address);
        }
        Ok(Self {
            addresses: Arc::new(addresses),
        })
    }

    pub fn contains(&self, address: &IpAddr) -> bool {
        self.addresses.contains(address)
    }

    pub fn is_empty(&self) -> bool {
        self.addresses.is_empty()
    }
}

fn fingerprint(digest: &[u8; 32]) -> String {
    digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn validate_key_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect API key file {}", path.display()))?;
    anyhow::ensure!(metadata.file_type().is_file(), "API key path must be a regular file");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        anyhow::ensure!(
            metadata.permissions().mode() & 0o077 == 0,
            "API key file {} is readable or writable by group/others; use chmod 600 (or a container secret mounted as 0400)",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_key_file(contents: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rkserve-auth-{}-{nonce}.json", std::process::id()));
        fs::write(&path, contents).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        }
        path
    }

    #[test]
    fn authenticates_and_enforces_scopes() {
        let path = temporary_key_file(
            r#"{"keys":[{"name":"console","key":"0123456789abcdef0123456789abcdef","scopes":["read","infer"]}]}"#,
        );
        let registry = AuthRegistry::load(&path).unwrap();
        fs::remove_file(path).unwrap();
        let principal = registry
            .authenticate("0123456789abcdef0123456789abcdef")
            .unwrap();
        assert_eq!(principal.name, "console");
        assert_eq!(principal.role, KeyRole::Management);
        assert!(principal.allows(SCOPE_READ));
        assert!(!principal.allows(SCOPE_CONTROL));
        assert!(principal.allows_path("/api/v1/plugins"));
        assert!(registry.authenticate("wrong").is_none());
    }

    #[test]
    fn api_role_is_limited_to_inference_jobs() {
        let rejected = temporary_key_file(
            r#"{"keys":[{"name":"inference","role":"api","key":"0123456789abcdef0123456789abcdef","scopes":["read","infer"]}]}"#,
        );
        assert!(AuthRegistry::load(&rejected).is_err());
        fs::remove_file(&rejected).unwrap();

        let path = temporary_key_file(
            r#"{"keys":[{"name":"inference","role":"api","key":"0123456789abcdef0123456789abcdef","scopes":["infer"]}]}"#,
        );
        let registry = AuthRegistry::load(&path).unwrap();
        fs::remove_file(path).unwrap();
        let principal = registry
            .authenticate("0123456789abcdef0123456789abcdef")
            .unwrap();
        assert_eq!(principal.role, KeyRole::Api);
        assert!(principal.allows_path("/api/v1/auth/whoami"));
        assert!(principal.allows_path("/api/v1/plugins/yolo26/jobs/vision.detect"));
        assert!(!principal.allows_path("/api/v1/plugins"));
        assert!(!principal.allows_path("/api/v1/events"));
        assert!(!principal.allows_path("/api/v1/scheduler/allocations"));
    }

    #[test]
    fn disabled_keys_cannot_authenticate() {
        let path = temporary_key_file(
            r#"{"keys":[
                {"name":"live","key":"0123456789abcdef0123456789abcdef","scopes":["read"]},
                {"name":"retired","key":"fedcba9876543210fedcba9876543210","scopes":["*"],"enabled":false}
            ]}"#,
        );
        let registry = AuthRegistry::load(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert!(registry.authenticate("0123456789abcdef0123456789abcdef").is_some());
        assert!(registry.authenticate("fedcba9876543210fedcba9876543210").is_none());
    }

    #[test]
    fn rejects_file_with_no_enabled_keys() {
        let path = temporary_key_file(
            r#"{"keys":[{"name":"retired","key":"0123456789abcdef0123456789abcdef","scopes":["*"],"enabled":false}]}"#,
        );
        assert!(AuthRegistry::load(&path).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_placeholder_and_unknown_scope() {
        let placeholder = temporary_key_file(
            r#"{"keys":[{"name":"console","key":"change-me-change-me-change-me-change-me","scopes":["*"]}]}"#,
        );
        assert!(AuthRegistry::load(&placeholder).is_err());
        fs::remove_file(placeholder).unwrap();

        let scope = temporary_key_file(
            r#"{"keys":[{"name":"console","key":"0123456789abcdef0123456789abcdef","scopes":["admin"]}]}"#,
        );
        assert!(AuthRegistry::load(&scope).is_err());
        fs::remove_file(scope).unwrap();
    }
}
