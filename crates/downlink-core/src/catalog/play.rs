//! Play-plan resolution from SQLite. No network: the plan names the URL, per-file headers, and
//! whether the channel needs a play-time redirect resolve.

use std::collections::HashMap;

use crate::auth::{AuthProvider, SecretString};
use crate::error::{AppError, Result};
use crate::redact::RedactionSet;
use crate::source::dispatcharr::{build_live_plan, normalise_base_url};
use crate::source::{ChannelKey, PlayPlan, PlayProfile, SourceKind, Surface};
use crate::store::Store;

pub async fn resolve_play_plan(
    store: &Store,
    auth: &dyn AuthProvider,
    channel_id: i64,
) -> Result<PlayPlan> {
    let ch = store
        .channel(channel_id)
        .await?
        .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
    if ch.is_drm {
        return Err(AppError::media("DRM — not supported"));
    }
    let src = store.source(ch.source_id).await?;
    match SourceKind::parse(&src.kind) {
        Some(SourceKind::Dispatcharr) => {
            let base = src
                .base_url
                .as_deref()
                .ok_or_else(|| AppError::local("This source has no server address."))?;
            let base = normalise_base_url(base)?;
            let (hname, secret) = auth.auth_header(&src).await?;
            let key = ChannelKey::from_row(&ch);
            let mut set = RedactionSet::new();
            set.add(secret.expose());
            build_live_plan(&base, &key, Some((hname, secret)), set)
        }
        Some(SourceKind::M3u) => {
            let url = ch
                .play_url
                .clone()
                .ok_or_else(|| AppError::stream("This channel has no stream address."))?;
            let hdrs: HashMap<String, String> = ch
                .play_headers
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?
                .unwrap_or_default();
            let mut set = RedactionSet::new();
            let headers = hdrs
                .into_iter()
                .map(|(k, v)| {
                    set.add(v.clone());
                    (k, SecretString::new(v))
                })
                .collect();
            Ok(PlayPlan {
                url,
                headers,
                surface: Surface::Direct,
                profile: PlayProfile::Live,
                session: None,
                redact: set,
                needs_redirect_resolve: false,
                unknown_profile: false,
            })
        }
        _ => Err(AppError::stream(
            "Playing from this kind of source is not available yet.",
        )),
    }
}
