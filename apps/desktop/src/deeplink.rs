//! `downlink://` deep links, routed to the UI as `app://deeplink`. Only a closed set of shapes is
//! accepted and only validated fields are forwarded (never the raw URL, which could carry anything).
//!
//! - `downlink://channel/<uuid>`
//! - `downlink://play?channel=<uuid>[&programme=<channelId>:<startUtc>][&start=<seconds>]`

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DeepLink {
    Channel {
        uuid: String,
    },
    Play {
        channel: Option<String>,
        programme: Option<String>,
        start: Option<f64>,
    },
}

fn valid_uuid(s: &str) -> bool {
    (8..=40).contains(&s.len()) && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

fn valid_programme(s: &str) -> bool {
    s.split_once(':').is_some_and(|(a, b)| {
        !a.is_empty()
            && !b.is_empty()
            && a.bytes().all(|c| c.is_ascii_digit())
            && b.bytes().all(|c| c.is_ascii_digit())
    })
}

pub fn parse(raw: &str) -> Option<DeepLink> {
    let u = url::Url::parse(raw).ok()?;
    if u.scheme() != "downlink" {
        return None;
    }
    match u.host_str()? {
        "channel" => {
            let uuid = u.path().trim_matches('/');
            valid_uuid(uuid).then(|| DeepLink::Channel {
                uuid: uuid.to_owned(),
            })
        }
        "play" => {
            let mut channel = None;
            let mut programme = None;
            let mut start = None;
            for (k, v) in u.query_pairs() {
                match k.as_ref() {
                    "channel" if valid_uuid(&v) => channel = Some(v.into_owned()),
                    "programme" if valid_programme(&v) => programme = Some(v.into_owned()),
                    "start" => start = v.parse::<f64>().ok().filter(|s| s.is_finite() && *s >= 0.0),
                    _ => {}
                }
            }
            (channel.is_some() || programme.is_some()).then_some(DeepLink::Play {
                channel,
                programme,
                start,
            })
        }
        _ => None,
    }
}

/// Parse and emit `app://deeplink`; bad links are dropped with a debug log (no URL in it).
pub fn forward<'a>(app: &AppHandle, urls: impl IntoIterator<Item = &'a str>) {
    for raw in urls {
        match parse(raw) {
            Some(link) => {
                tracing::info!(
                    "deep link: {}",
                    match &link {
                        DeepLink::Channel { .. } => "channel",
                        DeepLink::Play { .. } => "play",
                    }
                );
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
                let _ = app.emit("app://deeplink", link);
            }
            None => tracing::debug!("ignored an invalid deep link"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "3d949869-3e5b-44b5-9ebd-cbf422eb2611";

    #[test]
    fn channel_link() {
        assert_eq!(
            parse(&format!("downlink://channel/{U}")),
            Some(DeepLink::Channel { uuid: U.into() })
        );
        assert_eq!(parse("downlink://channel/not a uuid"), None);
        assert_eq!(parse("downlink://channel/"), None);
    }

    #[test]
    fn play_link() {
        let l = parse(&format!(
            "downlink://play?channel={U}&programme=12:1735725600&start=30.5"
        ));
        assert_eq!(
            l,
            Some(DeepLink::Play {
                channel: Some(U.into()),
                programme: Some("12:1735725600".into()),
                start: Some(30.5)
            })
        );
        assert_eq!(parse("downlink://play"), None);
        assert_eq!(parse("downlink://play?programme=../etc"), None);
    }

    #[test]
    fn rejects_other_schemes_and_hosts() {
        assert_eq!(parse("https://channel/x"), None);
        assert_eq!(parse("downlink://admin/panel"), None);
        assert_eq!(parse("downlink://play?url=http://evil/live/u/p/1.ts"), None);
    }
}
