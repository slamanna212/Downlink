//! `AppError` (PLAN §2.5): machine `code`, human `message`, optional `retry_after`.
//!
//! Every message is passed through [`crate::redact`] at construction, so an `AppError` can never
//! carry a credential to the UI, a log line or the diagnostics bundle.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::redact::{self, RedactionSet};

/// Error taxonomy. Only `Auth` and version-related `Server` errors block (see `blocking`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorCode {
    /// Bad or missing credentials, forbidden.
    Auth,
    /// Unreachable, DNS, TLS, timeout.
    Network,
    /// 5xx, unsupported server version, malformed server payloads.
    Server,
    /// Provider failure, no sources, connection limit, catch-up refused.
    Stream,
    /// Codec / format problems (reported by the player).
    Media,
    /// Disk, database, keychain.
    Local,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::Network => "network",
            Self::Server => "server",
            Self::Stream => "stream",
            Self::Media => "media",
            Self::Local => "local",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    /// Seconds the caller should wait before retrying (from `Retry-After`).
    #[serde(default)]
    pub retry_after: Option<u64>,
    /// True for the only failures that stop the app from working: auth, and unsupported server
    /// version (PLAN §2.5). Everything else degrades.
    #[serde(default)]
    pub blocking: bool,
    /// Source that requires a new trust decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_id: Option<std::num::NonZeroI64>,
    /// Optional detail for stream failures (PLAN §4.9): `limit`, `codec` or `engine`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Box<str>>,
    /// Codec name when `reason` is `codec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<String>,
    /// Short id to quote in bug reports when `reason` is `engine`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crash_id: Option<String>,
}

pub type Result<T, E = AppError> = std::result::Result<T, E>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl AsRef<str>) -> Self {
        Self {
            code,
            message: redact::redact(message.as_ref()),
            retry_after: None,
            blocking: code == ErrorCode::Auth,
            source_id: None,
            reason: None,
            codec: None,
            crash_id: None,
        }
    }

    /// Like [`AppError::new`] but also scrubs literal secrets the caller knows about.
    pub fn with_secrets(code: ErrorCode, message: impl AsRef<str>, secrets: &RedactionSet) -> Self {
        Self {
            code,
            message: secrets.redact(message.as_ref()),
            retry_after: None,
            blocking: code == ErrorCode::Auth,
            source_id: None,
            reason: None,
            codec: None,
            crash_id: None,
        }
    }

    pub fn auth(m: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::Auth, m)
    }
    pub fn network(m: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::Network, m)
    }
    pub fn server(m: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::Server, m)
    }
    pub fn stream(m: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::Stream, m)
    }
    pub fn media(m: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::Media, m)
    }
    pub fn local(m: impl AsRef<str>) -> Self {
        Self::new(ErrorCode::Local, m)
    }

    /// Server version below the supported minimum: a blocking `server` error.
    pub fn unsupported_version(found: &str, minimum: &str) -> Self {
        let mut e = Self::server(format!(
            "This server runs version {found}; Downlink needs {minimum} or newer."
        ));
        e.blocking = true;
        e
    }

    /// Attach the §4.9 detail fields (all optional).
    pub fn with_detail(
        mut self,
        reason: &str,
        codec: Option<String>,
        crash_id: Option<String>,
    ) -> Self {
        self.reason = Some(reason.into());
        self.codec = codec;
        self.crash_id = crash_id;
        self
    }

    pub fn source_id(&self) -> Option<i64> {
        self.source_id.map(std::num::NonZeroI64::get)
    }

    pub(crate) fn with_source_id(mut self, source_id: Option<i64>) -> Self {
        self.source_id = source_id.and_then(std::num::NonZeroI64::new);
        self
    }

    pub fn with_retry_after(mut self, secs: Option<u64>) -> Self {
        self.retry_after = secs;
        self
    }

    /// Map an HTTP status (and optional `Retry-After`) to an error.
    pub fn from_status(status: u16, retry_after: Option<u64>, what: &str) -> Self {
        let (code, msg) = match status {
            401 => (
                ErrorCode::Auth,
                "The server rejected the credentials.".to_owned(),
            ),
            403 => (
                ErrorCode::Auth,
                "This account is not allowed to do that.".to_owned(),
            ),
            404 => (
                ErrorCode::Server,
                format!("The server does not have {what}."),
            ),
            408 | 504 => (
                ErrorCode::Network,
                "The server took too long to answer.".to_owned(),
            ),
            429 => (
                ErrorCode::Server,
                "The server is rate limiting requests.".to_owned(),
            ),
            500..=599 => (
                ErrorCode::Server,
                format!("The server had a problem ({status})."),
            ),
            _ => (
                ErrorCode::Server,
                format!("The server answered {status} for {what}."),
            ),
        };
        Self::new(code, msg).with_retry_after(retry_after)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AppError {}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        // Drop the URL first: it may carry credentials, and redaction is a second line of defence.
        let e = e.without_url();
        if let Some(error) = crate::tls::certificate_error(&e) {
            return error;
        }
        if let Some(status) = e.status() {
            return Self::from_status(status.as_u16(), None, "that resource");
        }
        if e.is_timeout() {
            return Self::network("The server took too long to answer.");
        }
        if e.is_connect() {
            return Self::network("Could not connect to the server.");
        }
        if e.is_decode() || e.is_body() {
            return Self::server("The server sent a response Downlink could not read.");
        }
        if e.is_builder() || e.is_request() {
            return Self::network(format!("The request could not be sent: {e}"));
        }
        Self::network(format!("Network error: {e}"))
    }
}

impl From<sqlx::Error> for AppError {
    fn from(e: sqlx::Error) -> Self {
        match e {
            sqlx::Error::PoolTimedOut => Self::local("The local database is busy; try again."),
            sqlx::Error::Io(_) => Self::local("Could not read or write the local database."),
            other => Self::local(format!("Local database error: {other}")),
        }
    }
}

/// Shown when the database holds migrations this build does not know.
pub const NEWER_LIBRARY: &str = "This library was created by a newer version of Downlink.";

impl From<sqlx::migrate::MigrateError> for AppError {
    fn from(e: sqlx::migrate::MigrateError) -> Self {
        match e {
            sqlx::migrate::MigrateError::VersionMissing(_)
            | sqlx::migrate::MigrateError::VersionTooNew(..) => {
                let mut err = Self::local(NEWER_LIBRARY);
                err.blocking = true;
                err
            }
            other => Self::local(format!("Local database migration failed: {other}")),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        // Never echo the parser's message: type-mismatch errors quote the offending value, and the
        // body may hold credentials (e.g. provider account blobs). Position only.
        Self::server(format!(
            "The server sent data Downlink could not understand (line {}, column {}).",
            e.line(),
            e.column()
        ))
    }
}

impl From<keyring::Error> for AppError {
    fn from(e: keyring::Error) -> Self {
        Self::local(format!("The system keychain reported an error: {e}"))
    }
}

impl From<url::ParseError> for AppError {
    fn from(e: url::ParseError) -> Self {
        Self::local(format!("Invalid URL: {e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::local(format!("File error: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_is_redacted_on_construction() {
        let e = AppError::stream("failed http://p.test/live/alice/hunter2/1.ts");
        assert!(!e.message.contains("hunter2"));
        assert!(e.message.contains("/live/***/***/"));
    }

    #[test]
    fn serialises_for_tauri() {
        let e = AppError::from_status(429, Some(30), "channels");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "server");
        assert_eq!(v["retryAfter"], 30);
        assert!(v["message"].is_string());
        let back: AppError = serde_json::from_value(v).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn retry_after_is_null_when_none() {
        let v = serde_json::to_value(AppError::network("x")).unwrap();
        assert!(v["retryAfter"].is_null());
    }

    #[test]
    fn status_mapping() {
        assert_eq!(AppError::from_status(401, None, "x").code, ErrorCode::Auth);
        assert!(AppError::from_status(401, None, "x").blocking);
        assert_eq!(AppError::from_status(403, None, "x").code, ErrorCode::Auth);
        assert_eq!(
            AppError::from_status(404, None, "x").code,
            ErrorCode::Server
        );
        assert_eq!(
            AppError::from_status(503, None, "x").code,
            ErrorCode::Server
        );
        assert!(!AppError::from_status(503, None, "x").blocking);
        assert_eq!(
            AppError::from_status(504, None, "x").code,
            ErrorCode::Network
        );
    }

    #[test]
    fn only_auth_and_version_block() {
        assert!(AppError::auth("a").blocking);
        assert!(AppError::unsupported_version("0.30.0", "0.32.0").blocking);
        assert!(!AppError::network("n").blocking);
        assert!(!AppError::stream("s").blocking);
        assert!(!AppError::local("l").blocking);
    }

    #[test]
    fn sqlx_maps_to_local() {
        let e: AppError = sqlx::Error::PoolTimedOut.into();
        assert_eq!(e.code, ErrorCode::Local);
    }

    #[tokio::test]
    async fn reqwest_connect_error_is_network_and_url_free() {
        let err = reqwest::Client::new()
            .get("http://127.0.0.1:1/live/alice/hunter2/x?password=pw")
            .send()
            .await
            .unwrap_err();
        let e: AppError = err.into();
        assert_eq!(e.code, ErrorCode::Network);
        assert!(
            !e.message.contains("hunter2") && !e.message.contains("pw"),
            "{}",
            e.message
        );
    }

    #[test]
    fn secrets_set_is_applied() {
        let mut s = RedactionSet::new();
        s.add("topsecretvalue");
        let e = AppError::with_secrets(ErrorCode::Stream, "bad topsecretvalue here", &s);
        assert_eq!(e.message, "bad *** here");
    }
}
