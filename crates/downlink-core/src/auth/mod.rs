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
}

impl SecretKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "api_key",
            Self::Password => "password",
            Self::RefreshToken => "refresh_token",
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
