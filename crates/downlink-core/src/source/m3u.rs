//! M3U playlist source (PLAN §6.2/6.3): a playlist URL or local file, plus an optional XMLTV
//! guide (configured, or the playlist header's `url-tvg`). Stream URLs, per-file headers and the
//! catch-up template are stored per channel; the player never learns where a URL came from.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AccountInfo, Caps, ChannelKey, ChannelPage, GuideFeed, Page, PlayPlan, PlayProfile, Programme,
    SessionId, Source, SourceChannel, SourceGroup, SourceId, SourceKind, Surface, TimeWindow,
};
use crate::catchup::template::{CatchupTemplate, ExpandCtx, expand};
use crate::error::{AppError, Result};
use crate::m3u::load::{Fetcher, HttpFetcher, Location, load};
use crate::m3u::{M3uEntry, Playlist};
use crate::normalise::{stable_ids, synthetic_id};
use crate::redact::RedactionSet;

pub struct M3uSource {
    id: std::sync::atomic::AtomicI64,
    location: Location,
    /// Configured XMLTV URL(s); falls back to the header's `url-tvg` list.
    xmltv: Vec<String>,
    fetcher: Arc<dyn Fetcher>,
    insecure_tls: bool,
    cache: tokio::sync::Mutex<Option<Arc<Playlist>>>,
    any_catchup: AtomicBool,
    has_guide: AtomicBool,
    /// Set when a verified probe is adopted: the next `connect` reuses its loaded copy once.
    reuse_once: AtomicBool,
}

impl std::fmt::Debug for M3uSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("M3uSource(***)") // the location can carry credentials
    }
}

fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_default()
}

impl M3uSource {
    pub fn new(
        id: SourceId,
        location: Location,
        xmltv: Vec<String>,
        insecure_tls: bool,
    ) -> Result<Self> {
        Ok(Self::with_fetcher(
            id,
            location,
            xmltv,
            Arc::new(HttpFetcher::new(insecure_tls)?),
            insecure_tls,
        ))
    }

    pub fn with_fetcher(
        id: SourceId,
        location: Location,
        xmltv: Vec<String>,
        fetcher: Arc<dyn Fetcher>,
        insecure_tls: bool,
    ) -> Self {
        Self {
            id: std::sync::atomic::AtomicI64::new(id.0),
            has_guide: AtomicBool::new(!xmltv.is_empty()),
            location,
            xmltv,
            fetcher,
            insecure_tls,
            cache: tokio::sync::Mutex::new(None),
            any_catchup: AtomicBool::new(false),
            reuse_once: AtomicBool::new(false),
        }
    }

    /// A probe built before the row exists can be adopted once it has an id.
    pub fn set_id(&self, id: SourceId) {
        self.id.store(id.0, Ordering::Relaxed);
        self.reuse_once.store(true, Ordering::Relaxed);
    }

    async fn playlist(&self, refresh: bool) -> Result<Arc<Playlist>> {
        let mut g = self.cache.lock().await;
        // The first refresh after adoption reuses the copy `add_source` just verified; every
        // later refresh re-reads the playlist.
        let reuse = g.is_some() && self.reuse_once.swap(false, Ordering::Relaxed);
        if (refresh && !reuse) || g.is_none() {
            let (pl, st) = load(&self.location, self.fetcher.as_ref()).await?;
            tracing::info!(
                entries = pl.entries.len(),
                dropped = st.parse.dropped,
                "playlist loaded"
            );
            if !pl.header.tvg_urls.is_empty() {
                self.has_guide.store(true, Ordering::Relaxed);
            }
            let any = pl
                .entries
                .iter()
                .any(|e| CatchupTemplate::from_m3u(&e.attrs, &pl.header, None).is_some());
            self.any_catchup.store(any, Ordering::Relaxed);
            *g = Some(Arc::new(pl));
        }
        g.clone()
            .ok_or_else(|| AppError::local("The playlist is not loaded."))
    }

    fn to_channel(&self, e: &M3uEntry, id: String, pl: &Playlist) -> SourceChannel {
        let tpl = CatchupTemplate::from_m3u(&e.attrs, &pl.header, None);
        let headers = e.aux.http_headers();
        SourceChannel {
            ext_id: id,
            number: e.tvg_chno(),
            name: e.name.clone(),
            group_ext_id: e.group.clone(),
            tvg_id: e.tvg_id().map(str::to_owned),
            tvg_name: e.tvg_name().map(str::to_owned),
            logo_key: e.logo().map(str::to_owned),
            is_radio: e.is_radio(),
            is_drm: e.aux.is_drm(),
            is_catchup: tpl.is_some(),
            catchup_days: tpl.as_ref().map_or(0, |t| t.days),
            catchup_template: tpl.map(|t| t.to_json()),
            play_url: Some(e.url.clone()),
            play_headers: (!headers.is_empty()).then(|| {
                let m: serde_json::Map<String, Value> = headers
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();
                Value::Object(m).to_string()
            }),
            ..Default::default()
        }
    }

    fn plan_for(url: String, headers_json: Option<&str>, profile: PlayProfile) -> PlayPlan {
        let hdrs: Vec<(String, String)> = headers_json
            .and_then(|j| serde_json::from_str::<serde_json::Map<String, Value>>(j).ok())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| Some((k, v.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let mut redact = RedactionSet::new();
        redact.add(url.clone());
        for (_, v) in &hdrs {
            redact.add(v.clone());
        }
        let hls = url.contains(".m3u8");
        PlayPlan {
            url,
            headers: hdrs
                .into_iter()
                .map(|(k, v)| (k, crate::auth::SecretString::new(v)))
                .collect(),
            surface: if hls { Surface::Hls } else { Surface::Direct },
            profile,
            session: None,
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        }
    }
}

#[async_trait]
impl Source for M3uSource {
    fn id(&self) -> SourceId {
        SourceId(self.id.load(Ordering::Relaxed))
    }

    fn kind(&self) -> SourceKind {
        SourceKind::M3u
    }

    fn caps(&self) -> Caps {
        let mut c = Caps::LIVE | Caps::FAVOURITES;
        if self.has_guide.load(Ordering::Relaxed) {
            c |= Caps::EPG;
        }
        if self.any_catchup.load(Ordering::Relaxed) {
            c |= Caps::CATCHUP;
        }
        c
    }

    /// (Re)load the playlist so a bad address or file fails here, with a plain sentence.
    async fn connect(&self) -> Result<AccountInfo> {
        self.playlist(true).await?;
        Ok(AccountInfo::default())
    }

    /// Everything as one page; ids are stable across refreshes (tvg-id, else a name/group/host hash).
    async fn channels(&self, _page: Page) -> Result<ChannelPage> {
        let pl = self.playlist(false).await?;
        let base: Vec<String> = pl
            .entries
            .iter()
            .map(|e| {
                synthetic_id(
                    e.tvg_id(),
                    &e.name,
                    e.group.as_deref().unwrap_or(""),
                    &host_of(&e.url),
                )
            })
            .collect();
        let ids = stable_ids(&base);
        let channels: Vec<SourceChannel> = pl
            .entries
            .iter()
            .zip(ids)
            .map(|(e, id)| self.to_channel(e, id, &pl))
            .collect();
        Ok(ChannelPage {
            count: Some(channels.len() as u64),
            channels,
            has_next: false,
        })
    }

    async fn groups(&self) -> Result<Vec<SourceGroup>> {
        let pl = self.playlist(false).await?;
        let mut seen = std::collections::HashSet::new();
        Ok(pl
            .entries
            .iter()
            .filter_map(|e| e.group.clone())
            .filter(|g| seen.insert(g.clone()))
            .map(|g| SourceGroup {
                ext_id: g.clone(),
                name: g,
            })
            .collect())
    }

    async fn guide(&self, _window: TimeWindow) -> Result<Vec<Programme>> {
        Ok(Vec::new()) // the guide comes from `guide_feed` (XMLTV)
    }

    async fn guide_feed(&self) -> Result<Option<GuideFeed>> {
        let urls: Vec<String> = if self.xmltv.is_empty() {
            self.playlist(false).await?.header.tvg_urls.clone()
        } else {
            self.xmltv.clone()
        };
        let mut last_err = None;
        for u in urls {
            let got = match Location::parse(&u) {
                Ok(Location::File(p)) => Ok(crate::m3u::load::Fetched::Local(p)),
                _ => self.fetcher.fetch(&u).await,
            };
            match got {
                Ok(file) => {
                    return Ok(Some(GuideFeed {
                        file,
                        default_tz: None,
                    }));
                }
                Err(e) => last_err = Some(e),
            }
        }
        match last_err {
            Some(e) => Err(e),
            None => Ok(None),
        }
    }

    async fn play(&self, ch: ChannelKey) -> Result<PlayPlan> {
        let url = ch
            .play_url
            .clone()
            .filter(|u| !u.is_empty())
            .ok_or_else(|| AppError::stream("This channel has no stream address."))?;
        Ok(Self::plan_for(
            url,
            ch.play_headers.as_deref(),
            PlayProfile::Live,
        ))
    }

    /// Template path: the URL is built from the channel's stored template and the programme row.
    async fn catchup(&self, ch: ChannelKey, p: &Programme) -> Result<PlayPlan> {
        let tpl = ch
            .catchup_template
            .as_deref()
            .and_then(CatchupTemplate::from_json)
            .ok_or_else(|| AppError::stream("This channel does not offer an archive."))?;
        let live = ch.play_url.clone().unwrap_or_default();
        let url = expand(
            &tpl,
            &ExpandCtx {
                live_url: &live,
                start_utc: p.start_utc,
                end_utc: p.end_utc,
                now_utc: crate::sync::now_utc(),
                channel_id: &ch.ext_id,
                catchup_id: &ch.ext_id,
                xc: None,
            },
        )?;
        Ok(Self::plan_for(
            url,
            ch.play_headers.as_deref(),
            PlayProfile::Seekable,
        ))
    }

    async fn stop_session(&self, _s: SessionId) -> Result<()> {
        Ok(())
    }

    async fn fetch_image(&self, url: &str) -> Result<crate::images::RawImage> {
        crate::images::fetch_public(url, self.insecure_tls).await
    }
}

#[async_trait]
impl crate::sessions::SessionStopper for M3uSource {
    async fn stop(&self, _source_id: i64, _kind: &str, _session_id: &str) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
impl crate::userstate::AppDataRemote for M3uSource {
    async fn get(&self, _key: &str) -> Result<Option<Value>> {
        Ok(None)
    }
    async fn put(&self, _key: &str, _value: &Value) -> Result<()> {
        Ok(())
    }
}
