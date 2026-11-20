//! Play plans (PLAN §6.1, §3.5). A plan is a URL plus per-file headers; the player never learns
//! where it came from. Building a plan never touches a stream endpoint; the single exception is
//! [`resolve_redirect`], used only at play time for redirect-profile channels.

use std::fmt;
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, LOCATION, RETRY_AFTER};
use serde::Serialize;

use crate::auth::SecretString;
use crate::error::{AppError, Result};
use crate::redact::{self, RedactionSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Surface {
    Ts,
    Hls,
    Direct,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PlayProfile {
    Live,
    Seekable,
}

/// A catch-up / HLS session id; persisted in `open_session` for crash cleanup.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

impl fmt::Debug for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SessionId({})", redact::MASK)
    }
}

#[derive(Clone)]
pub struct PlayPlan {
    pub url: String,
    /// Passed to mpv as per-file options, never global (CLAUDE.md rule 5).
    pub headers: Vec<(String, SecretString)>,
    pub surface: Surface,
    pub profile: PlayProfile,
    pub session: Option<SessionId>,
    pub redact: RedactionSet,
    /// Redirect-profile channel: resolve with [`resolve_redirect`] at play time, then give mpv the
    /// `Location` and no headers (see [`PlayPlan::with_location`]).
    pub needs_redirect_resolve: bool,
    /// The channel's stream profile is unknown (played like Redirect, fail safe).
    pub unknown_profile: bool,
}

impl PlayPlan {
    /// Play-time resolution for Redirect/Unknown channels (CLAUDE.md rule 5):
    /// - 3xx: mpv gets the `Location` with no headers.
    /// - 2xx: the channel was really a proxy. The response is dropped at once and the original
    ///   URL plus header go to mpv. One extra client registration happened; logged at info when
    ///   the profile was unknown.
    pub async fn resolve_at_play(self, insecure_tls: bool) -> Result<PlayPlan> {
        if !self.needs_redirect_resolve {
            return Ok(self);
        }
        match probe_redirect(&self.url, &self.headers, insecure_tls).await? {
            RedirectOutcome::Redirect(loc) => Ok(self.with_location(&loc)),
            RedirectOutcome::Direct => {
                if self.unknown_profile {
                    tracing::info!(
                        "stream profile was unknown; one extra client registration was made to find out"
                    );
                }
                let mut p = self;
                p.needs_redirect_resolve = false;
                p.unknown_profile = false;
                Ok(p)
            }
        }
    }

    /// Replace the URL with a resolved redirect `Location`: no auth headers go to the provider.
    pub fn with_location(mut self, loc: &Location) -> Self {
        self.redact.add(loc.expose());
        self.url = loc.expose().to_owned();
        self.headers.clear();
        self.needs_redirect_resolve = false;
        self.unknown_profile = false;
        self
    }

    /// A printable, serialisable view with every secret masked.
    pub fn redacted(&self) -> RedactedPlan {
        let mut set = self.redact.clone();
        for (_, v) in &self.headers {
            set.add(v.expose());
        }
        RedactedPlan {
            url: set.redact(&self.url),
            headers: self
                .headers
                .iter()
                // every per-file header value is treated as secret
                .map(|(k, _)| (k.clone(), redact::MASK.to_owned()))
                .collect(),
            surface: self.surface,
            profile: self.profile,
            session: self.session.as_ref().map(|_| redact::MASK.to_owned()),
            needs_redirect_resolve: self.needs_redirect_resolve,
        }
    }
}

impl fmt::Debug for PlayPlan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.redacted(), f)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RedactedPlan {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub surface: Surface,
    pub profile: PlayProfile,
    pub session: Option<String>,
    pub needs_redirect_resolve: bool,
}

/// A redirect `Location`: a provider URL. Never logged; `Debug`/`Display` print a placeholder.
#[derive(Clone, PartialEq, Eq)]
pub struct Location(String);

impl Location {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Location(<redirect-url>)")
    }
}
impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redirect-url>")
    }
}

fn retry_after(h: &HeaderMap) -> Option<u64> {
    h.get(RETRY_AFTER)?.to_str().ok()?.trim().parse().ok()
}

/// What a play-time probe of a Redirect/Unknown channel found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectOutcome {
    Redirect(Location),
    /// 2xx: the endpoint serves the stream itself (proxy). The response was dropped unread.
    Direct,
}

/// Issue the play request with redirects **disabled** (PLAN §3.5 step 2). Use only at an actual
/// play; never as a probe.
pub async fn probe_redirect(
    url: &str,
    headers: &[(String, SecretString)],
    insecure_tls: bool,
) -> Result<RedirectOutcome> {
    let client = crate::tls::apply(reqwest::Client::builder(), insecure_tls)?
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(crate::user_agent())
        .timeout(Duration::from_secs(15))
        .build()?;
    let mut map = HeaderMap::new();
    for (k, v) in headers {
        let name = HeaderName::from_bytes(k.as_bytes())
            .map_err(|_| AppError::local("A request header name was invalid."))?;
        let mut val = HeaderValue::from_str(v.expose())
            .map_err(|_| AppError::local("A request header value was invalid."))?;
        val.set_sensitive(true);
        map.insert(name, val);
    }
    let resp = client.get(url).headers(map).send().await?;
    let status = resp.status();
    if status.is_redirection() {
        let loc = resp
            .headers()
            .get(LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::stream("The server redirected without saying where."))?;
        // Resolve relative redirects against the request URL.
        let base = reqwest::Url::parse(url)?;
        let abs = base.join(loc)?;
        return Ok(RedirectOutcome::Redirect(Location(abs.to_string())));
    }
    let ra = retry_after(resp.headers());
    // Dropping `resp` closes the connection without reading a body.
    drop(resp);
    if status.is_success() {
        return Ok(RedirectOutcome::Direct);
    }
    match status.as_u16() {
        401 | 403 => Err(AppError::from_status(status.as_u16(), ra, "this channel")),
        404 => Err(AppError::stream(
            "The server does not have this channel any more.",
        )),
        503 | 509 => Err(AppError::stream(
            "The provider has no free connection right now. Try again shortly.",
        )
        .with_retry_after(ra)),
        s => Err(
            AppError::stream(format!("The server could not start this channel ({s})."))
                .with_retry_after(ra),
        ),
    }
}

/// Strict variant: a 2xx is an error (the channel did not redirect).
pub async fn resolve_redirect(
    url: &str,
    headers: &[(String, SecretString)],
    insecure_tls: bool,
) -> Result<Location> {
    match probe_redirect(url, headers, insecure_tls).await? {
        RedirectOutcome::Redirect(l) => Ok(l),
        RedirectOutcome::Direct => Err(AppError::stream(
            "This channel did not redirect as expected. Its stream profile may have changed; refresh the channel list.",
        )),
    }
}
