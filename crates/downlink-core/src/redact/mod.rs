//! Credential redaction (PLAN §2.6, CLAUDE.md rules 7 and 8).
//!
//! Everything that can reach a log line, an `AppError` message, the debug panel or the
//! diagnostics bundle goes through [`redact`]. Redaction is pattern based (URL shapes, headers,
//! query credentials, JSON secrets) plus an explicit [`RedactionSet`] of literal secrets that a
//! caller knows about (an API key, a redirect `Location`).

mod tracing_writer;

use std::fmt;
use std::sync::LazyLock;

use regex::{Captures, Regex};

pub use tracing_writer::{RedactingMakeWriter, RedactingWriter};

/// Replacement text for any redacted value.
pub const MASK: &str = "***";

/// Compile a pattern that is a compile-time constant. A failure is a programming error caught by
/// the unit tests, so degrade to a never-matching regex instead of panicking in production.
fn rx(pattern: &str) -> Regex {
    Regex::new(pattern).unwrap_or_else(|_| {
        #[allow(clippy::unwrap_used, clippy::expect_used)]
        Regex::new(r"[^\s\S]").expect("static fallback regex is valid")
    })
}

/// `/live/<user>/<pass>/` and friends (Xtream), including `/timeshift/<u>/<p>/<dur>/<start>/<id>`.
static XTREAM_PATH: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)/(live|movie|series|timeshift)/([^/\s?#]+)/([^/\s?#]+)(/|\b)"));

/// HLS session tokens are bearer capabilities: `/proxy/hls/<token>/`.
static HLS_TOKEN: LazyLock<Regex> = LazyLock::new(|| rx(r"(?i)/proxy/hls/([^/\s?#]+)"));

/// `user:pass@host` userinfo in any URL.
static USERINFO: LazyLock<Regex> =
    LazyLock::new(|| rx(r"(?i)\b([a-z][a-z0-9+.\-]*://)[^/\s@?#]+@"));

/// Credential-carrying query parameters.
static QUERY_CRED: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r#"(?i)([?&;])(username|password|user|pass|pwd|token|api_key|apikey|access_token|refresh_token|auth|key|secret|sig|signature)=([^&\s"'#<>]*)"#,
    )
});

/// `Header-Name: value` / `Header-Name=value` for credential headers (to end of line / next quote).
static HEADER: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r#"(?i)\b(x-api-key|authorization|proxy-authorization|cookie|set-cookie|x-auth-token)(["']?\s*[:=]\s*["']?)([^\r\n"']*)"#,
    )
});

/// JSON-ish `"password":"…"` and debug-ish `password: "…"`.
static JSON_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    rx(
        r#"(?i)(["']?(?:password|passwd|secret|api_key|apikey|access|refresh|access_token|refresh_token|token)["']?\s*[:=]\s*)"([^"]*)""#,
    )
});

/// A set of literal secrets known to the caller (API key, JWT, redirect URL…).
#[derive(Clone, Default)]
pub struct RedactionSet {
    literals: Vec<String>,
}

impl RedactionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a literal secret. Very short values are ignored to avoid shredding logs.
    pub fn add(&mut self, secret: impl Into<String>) -> &mut Self {
        let s: String = secret.into();
        if s.len() >= 4 && !self.literals.contains(&s) {
            self.literals.push(s);
        }
        self
    }

    pub fn extend(&mut self, other: &RedactionSet) {
        for l in &other.literals {
            self.add(l.clone());
        }
    }

    pub fn is_empty(&self) -> bool {
        self.literals.is_empty()
    }

    /// Redact literals first (longest first), then run the pattern pass.
    pub fn redact(&self, input: &str) -> String {
        let mut out = input.to_owned();
        let mut lits: Vec<&String> = self.literals.iter().collect();
        lits.sort_by_key(|l| std::cmp::Reverse(l.len()));
        for l in lits {
            if out.contains(l.as_str()) {
                out = out.replace(l.as_str(), MASK);
            }
        }
        redact(&out)
    }
}

impl fmt::Debug for RedactionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RedactionSet({} literals)", self.literals.len())
    }
}

/// Pattern-based redaction. Idempotent.
pub fn redact(input: &str) -> String {
    let s = USERINFO.replace_all(input, |c: &Captures| format!("{}{}@", &c[1], MASK));
    let s = XTREAM_PATH.replace_all(&s, |c: &Captures| {
        format!("/{}/{MASK}/{MASK}{}", &c[1], &c[4])
    });
    let s = HLS_TOKEN.replace_all(&s, format!("/proxy/hls/{MASK}"));
    let s = QUERY_CRED.replace_all(&s, |c: &Captures| format!("{}{}={MASK}", &c[1], &c[2]));
    let s = HEADER.replace_all(&s, |c: &Captures| format!("{}{}{MASK}", &c[1], &c[2]));
    let s = JSON_SECRET.replace_all(&s, |c: &Captures| format!("{}\"{MASK}\"", &c[1]));
    s.into_owned()
}

/// Redact a URL that came from a redirect (`Location`): provider URLs are never logged, not even
/// their host. Path and query are dropped entirely.
pub fn redact_redirect_url(_url: &str) -> &'static str {
    "<redirect-url>"
}

/// Redact a header value by header name (value only; names are not secret).
pub fn redact_header_value<'a>(name: &str, value: &'a str) -> std::borrow::Cow<'a, str> {
    let n = name.to_ascii_lowercase();
    if matches!(
        n.as_str(),
        "x-api-key"
            | "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-auth-token"
    ) {
        std::borrow::Cow::Borrowed(MASK)
    } else {
        std::borrow::Cow::Owned(redact(value))
    }
}

/// Display wrapper that redacts at the logging boundary:
/// `tracing::info!(url = %Redacted(&url))`.
pub struct Redacted<T>(pub T);

impl<T: fmt::Display> fmt::Display for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&redact(&self.0.to_string()))
    }
}

impl<T: fmt::Debug> fmt::Debug for Redacted<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&redact(&format!("{:?}", self.0)))
    }
}

/// Convenience constructor.
pub fn redacted<T: fmt::Display>(v: T) -> Redacted<T> {
    Redacted(v)
}

#[cfg(test)]
mod tests;
