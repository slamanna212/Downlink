//! Credential templating (CLAUDE.md rule 7: secrets never land in SQLite).
//!
//! Anything in a stored URL or header that the redaction rules would mask (Xtream path
//! credentials, userinfo, `username`/`password`/`token` query parameters, credential headers) is
//! replaced by a placeholder such as `{dl:user}` before it is written to SQLite. The extracted
//! values live in a per-source [`Vault`], which the app persists in the OS keychain (one entry per
//! source, deduplicated: identical values share one placeholder, a differing value gets a short
//! hash suffix). Placeholders are resolved in Rust at play/fetch time only.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use regex::{Captures, Regex};
use sha2::{Digest, Sha256};

use crate::error::{AppError, Result};

static USERINFO: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)(://)([^/\s@?#]+)@").ok());
static XTREAM_PATH: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r#"(?i)(/(?:live|movie|series|timeshift)/)([^/\s?#"'<>]+)/([^/\s?#"'<>]+)"#).ok()
});
static HLS_TOKEN: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(?i)(/proxy/hls/)([^/\s?#"'<>]+)"#).ok());
static PLACEHOLDER: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\{dl:([a-z]+(?::[0-9a-f]{8})?)\}").ok());

fn is_placeholder(s: &str) -> bool {
    PLACEHOLDER
        .as_ref()
        .and_then(|re| re.find(s))
        .is_some_and(|m| m.as_str() == s)
}

fn short_hash(v: &str) -> String {
    Sha256::digest(v.as_bytes())
        .iter()
        .take(4)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The credential store for one source. Cheap to clone (shared).
#[derive(Clone, Default)]
pub struct Vault(Arc<Mutex<HashMap<String, String>>>);

impl std::fmt::Debug for Vault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Vault({} entries)",
            self.0.lock().map(|g| g.len()).unwrap_or(0)
        )
    }
}

impl Vault {
    pub fn new() -> Self {
        Self::default()
    }

    /// Placeholder for `value` under `base` (`user`, `pass`, `token`, `hdr`). The first distinct
    /// value gets `{dl:base}`; further distinct values get `{dl:base:<8 hex>}`.
    fn intern(&self, base: &str, value: &str) -> String {
        let Ok(mut g) = self.0.lock() else {
            return value.to_owned();
        };
        match g.get(base) {
            None => {
                g.insert(base.to_owned(), value.to_owned());
                format!("{{dl:{base}}}")
            }
            Some(v) if v == value => format!("{{dl:{base}}}"),
            Some(_) => {
                let key = format!("{base}:{}", short_hash(value));
                g.entry(key.clone()).or_insert_with(|| value.to_owned());
                format!("{{dl:{key}}}")
            }
        }
    }

    fn credential(&self, base: &str, value: &str) -> String {
        if value.is_empty() || is_placeholder(value) {
            return value.to_owned();
        }
        self.intern(base, value)
    }

    /// Replace credentials in a URL (or URL-bearing string) with placeholders. Idempotent.
    pub fn template_url(&self, url: &str) -> String {
        let mut s = url.to_owned();
        if let Some(re) = USERINFO.as_ref() {
            s = re
                .replace_all(&s, |c: &Captures| {
                    // Colons inside placeholders are part of the username, not separators.
                    let user_info = &c[2];
                    let separator = PLACEHOLDER
                        .as_ref()
                        .and_then(|re| re.find(user_info))
                        .filter(|p| p.start() == 0)
                        .map_or(0, |p| p.end());
                    let templated = match user_info[separator..].find(':') {
                        Some(i) => {
                            let (user, pass) = user_info.split_at(separator + i);
                            format!(
                                "{}:{}",
                                self.credential("user", user),
                                self.credential("pass", &pass[1..])
                            )
                        }
                        None => self.credential("user", user_info),
                    };
                    format!("{}{templated}@", &c[1])
                })
                .into_owned();
        }
        if let Some(re) = XTREAM_PATH.as_ref() {
            s = re
                .replace_all(&s, |c: &Captures| {
                    format!(
                        "{}{}/{}",
                        &c[1],
                        self.credential("user", &c[2]),
                        self.credential("pass", &c[3]),
                    )
                })
                .into_owned();
        }
        if let Some(re) = HLS_TOKEN.as_ref() {
            s = re
                .replace_all(&s, |c: &Captures| {
                    format!("{}{}", &c[1], self.credential("token", &c[2]))
                })
                .into_owned();
        }
        {
            let re = &*crate::redact::QUERY_CRED;
            s = re
                .replace_all(&s, |c: &Captures| {
                    let val = &c[3];
                    if val.is_empty() || is_placeholder(val) {
                        return c[0].to_owned();
                    }
                    let base = match c[2].to_ascii_lowercase().as_str() {
                        "username" | "user" => "user",
                        "password" | "pass" | "pwd" => "pass",
                        _ => "token",
                    };
                    format!("{}{}={}", &c[1], &c[2], self.intern(base, val))
                })
                .into_owned();
        }
        s
    }

    /// Template a header value when the header name is a credential header.
    pub fn template_header(&self, name: &str, value: &str) -> String {
        if crate::redact::is_credential_header(name) {
            self.credential("hdr", value)
        } else {
            self.template_url(value)
        }
    }

    /// Resolve placeholders; an unresolved one is an auth error (the saved credentials are gone),
    /// never a request with a literal placeholder.
    pub fn resolve(&self, s: &str) -> Result<String> {
        let Some(re) = PLACEHOLDER.as_ref() else {
            return Ok(s.to_owned());
        };
        let g = self
            .0
            .lock()
            .map_err(|_| AppError::local("The credential store is unavailable."))?;
        let mut missing = false;
        let out = re
            .replace_all(s, |c: &Captures| match g.get(&c[1]) {
                Some(v) => v.clone(),
                None => {
                    missing = true;
                    String::new()
                }
            })
            .into_owned();
        if missing {
            return Err(AppError::auth(
                "The saved credentials for this source are missing. Refresh the source.",
            ));
        }
        Ok(out)
    }

    pub fn export(&self) -> HashMap<String, String> {
        self.0.lock().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn load(&self, map: HashMap<String, String>) {
        if let Ok(mut g) = self.0.lock() {
            *g = map;
        }
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().map(|g| g.is_empty()).unwrap_or(true)
    }

    /// Serialise for the keychain.
    pub fn to_secret_json(&self) -> String {
        serde_json::to_string(&self.export()).unwrap_or_default()
    }

    pub fn load_secret_json(&self, s: &str) {
        if let Ok(m) = serde_json::from_str::<HashMap<String, String>>(s) {
            self.load(m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_every_credential_shape_and_roundtrips() {
        let v = Vault::new();
        let cases = [
            "http://u1:p1xx@iptv.test/auth/1.ts",
            "http://iptv.test/live/u1/p1xx/1.ts",
            "http://iptv.test/movie/u1/p1xx/2.mkv",
            "http://iptv.test/series/u1/p1xx/3.mp4",
            "http://iptv.test/timeshift/u1/p1xx/60/2026-07-01:13-00/1.ts",
            "http://iptv.test/get.php?username=u1&password=p1xx&type=m3u",
            "http://iptv.test/x.ts?token=tok12345&a=b",
        ];
        for c in cases {
            let t = v.template_url(c);
            assert!(!t.contains("p1xx") && !t.contains("tok12345"), "{c} -> {t}");
            assert!(
                !t.contains("u1:") && !t.contains("/u1/") && !t.contains("username=u1"),
                "{c} -> {t}"
            );
            assert!(t.contains("{dl:"), "{c} -> {t}");
            assert_eq!(v.resolve(&t).unwrap(), c, "roundtrip");
            assert_eq!(v.template_url(&t), t, "idempotent");
        }
        // identical values share placeholders: one user, one pass, one token in the vault
        let e = v.export();
        assert_eq!(e.get("user").map(String::as_str), Some("u1"));
        assert_eq!(e.get("pass").map(String::as_str), Some("p1xx"));
        assert_eq!(e.len(), 3, "{e:?}");
    }

    #[test]
    fn templates_tokens_and_headers_covered_by_redaction() {
        let v = Vault::new();
        let url = "https://u:colon:password@iptv.test/proxy/hls/session-token/index.m3u8?x-plex-token=plex-token;refresh_token=refresh-secret";
        let template = v.template_url(url);
        for secret in [
            "colon:password",
            "session-token",
            "=plex-token",
            "refresh-secret",
        ] {
            assert!(!template.contains(secret));
        }
        assert_eq!(v.resolve(&template).unwrap(), url);
        assert_eq!(v.template_url(&template), template);
        for header in ["X-Plex-Token", "Set-Cookie"] {
            let template = v.template_header(header, "header-secret");
            assert_eq!(v.template_header(header, &template), template);
            assert_eq!(v.resolve(&template).unwrap(), "header-secret");
            assert_eq!(
                crate::redact::redact_header_value(header, "header-secret"),
                "***"
            );
        }
        let mixed = v.template_url("https://iptv.test/live/{dl:user}/new-pass!/1.ts");
        assert!(!mixed.contains("new-pass!"));
        assert_eq!(
            v.resolve(&mixed).unwrap(),
            "https://iptv.test/live/u/new-pass!/1.ts"
        );
    }

    #[test]
    fn differing_values_get_hash_keys_and_plain_urls_are_untouched() {
        let v = Vault::new();
        let a = v.template_url("http://h.test/a?token=aaaaaaaa");
        let b = v.template_url("http://h.test/b?token=bbbbbbbb");
        assert!(
            a.contains("{dl:token}") && b.contains("{dl:token:"),
            "{a} {b}"
        );
        assert_eq!(v.resolve(&a).unwrap(), "http://h.test/a?token=aaaaaaaa");
        assert_eq!(v.resolve(&b).unwrap(), "http://h.test/b?token=bbbbbbbb");
        let clean = "http://h.test/live/1.ts?x=1";
        assert_eq!(v.template_url(clean), clean);
        assert_eq!(
            v.template_url("http://h.test/x?password="),
            "http://h.test/x?password=",
            "empty values stay"
        );
    }

    #[test]
    fn headers_and_catchup_source_templates() {
        let v = Vault::new();
        let h = v.template_header("Authorization", "Bearer abc.def.ghi");
        assert_eq!(h, "{dl:hdr}");
        assert_eq!(v.resolve(&h).unwrap(), "Bearer abc.def.ghi");
        assert_eq!(
            v.template_header("Referer", "http://r.test/"),
            "http://r.test/"
        );
        let t = v.template_url("http://p.test/timeshift.php?username=u&password=pppp&stream=1&utc={utc}&duration={duration:60}");
        assert!(
            t.contains("{utc}") && t.contains("{duration:60}") && !t.contains("pppp"),
            "{t}"
        );
    }

    #[test]
    fn unresolved_placeholder_is_an_auth_error() {
        let v = Vault::new();
        let e = v
            .resolve("http://h.test/live/{dl:user}/{dl:pass}/1.ts")
            .unwrap_err();
        assert_eq!(e.code, crate::ErrorCode::Auth);
        assert_eq!(v.resolve("no placeholders").unwrap(), "no placeholders");
    }

    #[test]
    fn persists_through_secret_json() {
        let v = Vault::new();
        v.template_url("http://h.test/live/u/ppppp/1.ts");
        let j = v.to_secret_json();
        let w = Vault::new();
        w.load_secret_json(&j);
        assert_eq!(w.resolve("{dl:user}/{dl:pass}").unwrap(), "u/ppppp");
        assert!(!format!("{v:?}").contains("ppppp"));
    }
}
