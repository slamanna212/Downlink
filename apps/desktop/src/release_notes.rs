//! Release notes for the in-app "What's new": the GitHub release body for the update's tag
//! (Markdown, edited by hand on GitHub), falling back to the notes in `latest.json`.
//! The UI never talks HTTP (rule 2), so it asks for them through `get_release_notes`.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Largest body we accept; release notes are a few KB.
const MAX_BODY_BYTES: usize = 256 * 1024;
const API_BASE: &str = "https://api.github.com";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseNotesDto {
    pub version: String,
    /// Markdown, or `None` when there is nothing to show.
    pub body: Option<String>,
    /// `github` (the release body) or `manifest` (the notes in latest.json).
    pub source: &'static str,
}

#[derive(Deserialize)]
struct ReleaseJson {
    body: Option<String>,
}

/// `(owner, repo)` from a `https://github.com/{owner}/{repo}/...` URL, so the repo is defined
/// once (the updater endpoints).
pub fn repo_from_url(url: &str) -> Option<(String, String)> {
    let u = url::Url::parse(url).ok()?;
    if u.host_str() != Some("github.com") {
        return None;
    }
    let mut seg = u.path_segments()?;
    let owner = seg.next().filter(|s| !s.is_empty())?;
    let repo = seg.next().filter(|s| !s.is_empty())?;
    Some((owner.to_owned(), repo.to_owned()))
}

/// Versions become a URL path segment, so only version-ish characters are accepted.
pub fn valid_version(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 64
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
}

/// `GET {api}/repos/{owner}/{repo}/releases/tags/v{version}`. Unauthenticated; any failure is an `Err`
/// string (never shown to the user, logged at debug by the caller).
pub async fn fetch_release_body(
    api_base: &str,
    owner: &str,
    repo: &str,
    version: &str,
) -> Result<Option<String>, String> {
    let url = format!("{api_base}/repos/{owner}/{repo}/releases/tags/v{version}");
    let client = reqwest::Client::builder()
        .user_agent("Downlink-updater")
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }
    let bytes = downlink_core::http::read_body(resp, MAX_BODY_BYTES, downlink_core::AppError::from)
        .await
        .map_err(|e| e.message)?;
    let rel: ReleaseJson = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(rel.body.filter(|b| !b.trim().is_empty()))
}

/// The GitHub body if it can be had, else `fallback` (the latest.json notes).
pub async fn release_notes(
    api_base: &str,
    repo: Option<(String, String)>,
    version: &str,
    fallback: Option<String>,
) -> ReleaseNotesDto {
    if let Some((owner, name)) = repo {
        match fetch_release_body(api_base, &owner, &name, version).await {
            Ok(Some(body)) => {
                return ReleaseNotesDto {
                    version: version.to_owned(),
                    body: Some(body),
                    source: "github",
                };
            }
            Ok(None) => {}
            Err(e) => tracing::debug!("release notes fetch failed: {e}"),
        }
    }
    ReleaseNotesDto {
        version: version.to_owned(),
        body: fallback.filter(|b| !b.trim().is_empty()),
        source: "manifest",
    }
}

pub async fn release_notes_default(version: &str, fallback: Option<String>) -> ReleaseNotesDto {
    let repo = repo_from_url(crate::updates::STABLE_ENDPOINT);
    release_notes(API_BASE, repo, version, fallback).await
}

/// Links in release notes open in the system browser; web links only.
pub fn external_link_allowed(url: &str) -> bool {
    url::Url::parse(url)
        .is_ok_and(|u| matches!(u.scheme(), "https" | "http") && u.host_str().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn repo_comes_from_endpoint() {
        assert_eq!(
            repo_from_url(crate::updates::STABLE_ENDPOINT),
            Some(("slamanna212".to_owned(), "Downlink".to_owned()))
        );
        assert_eq!(repo_from_url("https://example.invalid/a/b"), None);
    }

    #[test]
    fn version_and_link_validation() {
        assert!(valid_version("0.8.1-dev.4"));
        assert!(!valid_version("../x"));
        assert!(!valid_version(""));
        assert!(external_link_allowed("https://example.invalid/x"));
        assert!(!external_link_allowed("file:///etc/passwd"));
        assert!(!external_link_allowed("javascript:alert(1)"));
    }

    #[tokio::test]
    async fn uses_github_body() {
        let srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases/tags/v1.2.3"))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({ "tag_name": "v1.2.3", "body": "## Hi\n![x](https://example.invalid/a.png)" }),
            ))
            .mount(&srv)
            .await;
        let dto = release_notes(
            &srv.uri(),
            Some(("o".into(), "r".into())),
            "1.2.3",
            Some("old".into()),
        )
        .await;
        assert_eq!(dto.source, "github");
        assert_eq!(
            dto.body.as_deref(),
            Some("## Hi\n![x](https://example.invalid/a.png)")
        );
    }

    #[tokio::test]
    async fn falls_back_on_errors_and_empty_bodies() {
        let srv = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases/tags/v1.0.0"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&srv)
            .await;
        Mock::given(method("GET"))
            .and(path("/repos/o/r/releases/tags/v2.0.0"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "body": "  " })),
            )
            .mount(&srv)
            .await;
        for v in ["1.0.0", "2.0.0", "3.0.0"] {
            let dto = release_notes(
                &srv.uri(),
                Some(("o".into(), "r".into())),
                v,
                Some("manifest notes".into()),
            )
            .await;
            assert_eq!(dto.source, "manifest", "{v}");
            assert_eq!(dto.body.as_deref(), Some("manifest notes"));
        }
        let dto = release_notes(&srv.uri(), None, "1.0.0", None).await;
        assert_eq!(dto.body, None);
    }
}
