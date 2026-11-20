//! Secret storage (PLAN §2.6). Secrets live only in the OS keychain; SQLite stores a reference.
//! The store is behind a trait so tests (and headless CLI use) use [`MemorySecretStore`].

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use crate::error::{AppError, Result};

/// A secret value. `Debug`/`Display` never print it; read it with [`SecretString::expose`] only
/// at the point of use (building a header, a keychain write).
#[derive(Clone, PartialEq, Eq)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString(***)")
    }
}

impl fmt::Display for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(crate::redact::MASK)
    }
}

impl From<&str> for SecretString {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// What a keychain entry holds for a source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    ApiKey,
    Password,
    RefreshToken,
    /// An M3U playlist URL (may embed credentials).
    PlaylistUrl,
    /// An XMLTV guide URL (may embed credentials).
    XmltvUrl,
    /// The per-source credential vault (templated URL/header values, see `vault`).
    Credentials,
}

impl SecretKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Password => "password",
            Self::RefreshToken => "refresh_token",
            Self::PlaylistUrl => "playlist_url",
            Self::XmltvUrl => "xmltv_url",
            Self::Credentials => "credentials",
        }
    }
}

/// The reference string stored in `source.secret_ref` (never the secret).
pub fn secret_ref(source_uuid: &str, kind: SecretKind) -> String {
    format!("downlink/source/{source_uuid}/{}", kind.as_str())
}

/// Keychain access by reference. Methods are synchronous (OS keychain APIs are); call from
/// `spawn_blocking` on hot paths.
pub trait SecretStore: Send + Sync {
    fn set(&self, reference: &str, secret: &SecretString) -> Result<()>;
    fn get(&self, reference: &str) -> Result<Option<SecretString>>;
    fn delete(&self, reference: &str) -> Result<()>;
}

/// Service name under which entries are filed in the OS keychain.
pub const KEYRING_SERVICE: &str = "downlink";

/// Credential Manager / Keychain / Secret Service.
#[derive(Debug, Default, Clone)]
pub struct KeyringStore;

impl KeyringStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(reference: &str) -> Result<keyring::Entry> {
        Ok(keyring::Entry::new(KEYRING_SERVICE, reference)?)
    }
}

impl SecretStore for KeyringStore {
    fn set(&self, reference: &str, secret: &SecretString) -> Result<()> {
        Ok(Self::entry(reference)?.set_password(secret.expose())?)
    }

    fn get(&self, reference: &str) -> Result<Option<SecretString>> {
        match Self::entry(reference)?.get_password() {
            Ok(p) => Ok(Some(SecretString::new(p))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn delete(&self, reference: &str) -> Result<()> {
        match Self::entry(reference)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// In-memory implementation for tests and the headless CLI.
#[derive(Debug, Default)]
pub struct MemorySecretStore {
    map: Mutex<HashMap<String, SecretString>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, SecretString>>> {
        self.map
            .lock()
            .map_err(|_| AppError::local("The secret store is unavailable."))
    }
}

impl SecretStore for MemorySecretStore {
    fn set(&self, reference: &str, secret: &SecretString) -> Result<()> {
        self.lock()?.insert(reference.to_owned(), secret.clone());
        Ok(())
    }
    fn get(&self, reference: &str) -> Result<Option<SecretString>> {
        Ok(self.lock()?.get(reference).cloned())
    }
    fn delete(&self, reference: &str) -> Result<()> {
        self.lock()?.remove(reference);
        Ok(())
    }
}

/// Supplies the per-request auth header for a source: the API key from the keychain, or a JWT
/// access token from a live client (JWT sources). Lets play-plan resolution work for both.
#[async_trait::async_trait]
pub trait AuthProvider: Send + Sync {
    async fn auth_header(&self, source: &crate::store::SourceRow)
    -> Result<(String, SecretString)>;

    /// The source's credential vault (values behind `{dl:…}` placeholders in stored URLs).
    async fn credentials(
        &self,
        _source: &crate::store::SourceRow,
    ) -> Result<std::collections::HashMap<String, String>> {
        Ok(std::collections::HashMap::new())
    }
}

/// API-key auth read straight from the keychain (`source.secret_ref`).
pub struct ApiKeyAuth<'a>(pub &'a dyn SecretStore);

#[async_trait::async_trait]
impl AuthProvider for ApiKeyAuth<'_> {
    async fn auth_header(
        &self,
        source: &crate::store::SourceRow,
    ) -> Result<(String, SecretString)> {
        let secret = match source.secret_ref.as_deref() {
            Some(r) => self.0.get(r)?,
            None => None,
        };
        let secret = secret.ok_or_else(|| {
            AppError::auth("The saved credentials for this source were not found. Sign in again.")
        })?;
        Ok(("X-API-Key".to_owned(), secret))
    }

    async fn credentials(
        &self,
        source: &crate::store::SourceRow,
    ) -> Result<std::collections::HashMap<String, String>> {
        let r = secret_ref(&source.uuid, SecretKind::Credentials);
        Ok(self
            .0
            .get(&r)?
            .and_then(|s| serde_json::from_str(s.expose()).ok())
            .unwrap_or_default())
    }
}

/// Store a secret in the keychain and return the reference to persist in SQLite.
pub fn put_source_secret(
    secrets: &dyn SecretStore,
    source_uuid: &str,
    kind: SecretKind,
    secret: &SecretString,
) -> Result<String> {
    let r = secret_ref(source_uuid, kind);
    secrets.set(&r, secret)?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_string_never_prints() {
        let s = SecretString::new("hunter2hunter2");
        assert!(!format!("{s:?}").contains("hunter2"));
        assert!(!format!("{s}").contains("hunter2"));
        assert_eq!(s.expose(), "hunter2hunter2");
    }

    #[test]
    fn memory_store_roundtrip() {
        let st = MemorySecretStore::new();
        let r = secret_ref("uuid-1", SecretKind::ApiKey);
        assert_eq!(r, "downlink/source/uuid-1/api_key");
        assert!(st.get(&r).unwrap().is_none());
        st.set(&r, &SecretString::new("k")).unwrap();
        assert_eq!(st.get(&r).unwrap().unwrap().expose(), "k");
        st.delete(&r).unwrap();
        st.delete(&r).unwrap(); // idempotent
        assert!(st.get(&r).unwrap().is_none());
    }

    #[test]
    fn put_returns_reference_not_secret() {
        let st = MemorySecretStore::new();
        let r = put_source_secret(
            &st,
            "u",
            SecretKind::Password,
            &SecretString::new("topsecret"),
        )
        .unwrap();
        assert!(!r.contains("topsecret"));
        assert!(st.get(&r).unwrap().is_some());
    }
}
