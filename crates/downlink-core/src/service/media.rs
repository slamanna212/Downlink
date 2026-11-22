//! VOD and DVR verbs of the façade (PLAN M4). Local row ids are what the UI sees: VOD titles use
//! `vod_item.id`, recordings `recording.id`, rules `recording_rule.id`. Category ids encode the
//! source (`source_id * 1_000_000 + server id`) so several sources never collide.

use super::*;
use crate::source::{
    Caps, RecStatus, RecordRequest, RecordingInfo, RuleInfo, VodItem, VodKind, VodListQuery,
};
use crate::store::{RecordingRow, ResumeRow, VodRow};

const CAT_STRIDE: i64 = 1_000_000;
const VOD_CATS_ENTITY: &str = "vod_categories";

/// What `play_vod` hands the desktop.
#[derive(Debug)]
pub struct VodPlay {
    pub plan: PlayPlan,
    /// Seek here after loading (seconds), when `resume` was asked for and a position is saved.
    pub resume_secs: Option<i64>,
    pub title: String,
    pub kind: VodKind,
    pub id: i64,
    pub source_id: i64,
    pub duration_secs: Option<i64>,
}

/// What `play_recording` hands the desktop.
#[derive(Debug)]
pub struct RecordingPlay {
    pub plan: PlayPlan,
    pub resume_secs: Option<i64>,
    pub title: String,
    pub id: i64,
    pub in_progress: bool,
}

fn compose_cat(source_id: i64, ext: &str) -> i64 {
    use sha2::{Digest, Sha256};
    let n = ext.parse::<i64>().unwrap_or_else(|_| {
        let d = Sha256::digest(ext.as_bytes());
        i64::from(u32::from_be_bytes([d[0], d[1], d[2], d[3]]))
    });
    source_id * CAT_STRIDE + n.rem_euclid(CAT_STRIDE)
}

fn rating_of(r: Option<&str>) -> Option<f64> {
    let v: f64 = r?.trim().parse().ok()?;
    // 0 means "no rating" to every source; NaN/inf and out-of-scale values are garbage.
    (v > 0.0 && v <= 10.0).then_some(v)
}

fn genres_of(g: Option<&str>) -> Vec<String> {
    g.map(|g| {
        g.split(',')
            .map(|x| x.trim().to_owned())
            .filter(|x| !x.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

fn to_item(r: &VodRow) -> VodItem {
    VodItem {
        kind: VodKind::parse(&r.kind),
        ext_id: r.ext_id.clone(),
        uuid: r.uuid.clone(),
        parent_ext: r.parent_ext.clone(),
        title: r.name.clone(),
        container: r.container.clone(),
        tmdb_id: r.tmdb_id.clone(),
        imdb_id: r.imdb_id.clone(),
        ..Default::default()
    }
}

impl Service {
    // ---- VOD --------------------------------------------------------------------------------

    async fn caps_sources(&self, cap: Caps) -> Result<Vec<SourceRow>> {
        Ok(self
            .store
            .list_sources()
            .await?
            .into_iter()
            .filter(|s| Caps::from_db(s.caps).contains(cap))
            .collect())
    }

    async fn art(&self, source_id: i64, url: Option<&str>) -> Option<String> {
        self.images
            .register(source_id, url?, ImageSize::Detail)
            .await
            .ok()
    }

    fn category_of(&self, row: &VodRow) -> Option<i64> {
        row.category_ext
            .as_deref()
            .map(|c| compose_cat(row.source_id, c))
    }

    pub async fn list_vod_categories(&self, kind: VodKindArg) -> Result<Vec<VodCategoryDto>> {
        self.vod_categories(kind, true).await
    }

    /// Categories with items, read from the cache the catalog sync fills (no provider requests).
    /// `with_counts` is kept for the command signature: counts are always known from the cache.
    pub async fn vod_categories(
        &self,
        kind: VodKindArg,
        _with_counts: bool,
    ) -> Result<Vec<VodCategoryDto>> {
        let k = if kind == VodKindArg::Movie {
            VodKind::Movie
        } else {
            VodKind::Series
        };
        let mut out = Vec::new();
        for src in self.caps_sources(Caps::VOD).await? {
            // A source that has never completed a VOD category sync (e.g. upgraded install) gets
            // one now; afterwards only the catalog sync refreshes it.
            let synced = self
                .store
                .sync_state(src.id, VOD_CATS_ENTITY)
                .await?
                .is_some_and(|(ok, _)| ok.is_some());
            if !synced && let Err(e) = self.refresh_vod_categories(src.id).await {
                tracing::warn!(error = %e, "vod category refresh failed");
            }
            // Categories the server reports as empty are hidden (unknown counts stay listed).
            for c in self
                .store
                .vod_categories(src.id, k)
                .await?
                .into_iter()
                .filter(|c| c.count != Some(0))
            {
                let id = compose_cat(src.id, &c.ext_id);
                if let Ok(mut g) = self.vod_cats.lock() {
                    g.insert(id, (src.id, c.ext_id.clone()));
                }
                out.push(VodCategoryDto {
                    id,
                    name: c.name,
                    count: c.count.unwrap_or(0),
                    count_pending: false,
                });
            }
        }
        Ok(out)
    }

    /// Recompute and persist a source's non-empty VOD categories (movies and series). Called by
    /// the catalog sync; on failure the previous cache is kept and the error recorded.
    pub(super) async fn refresh_vod_categories(&self, source_id: i64) -> Result<()> {
        let r = async {
            let client = self.client(source_id).await?;
            for k in [VodKind::Movie, VodKind::Series] {
                let categories = client.vod_categories(k).await?;
                self.store
                    .replace_vod_categories(source_id, k, &categories)
                    .await?;
            }
            Ok::<(), crate::AppError>(())
        }
        .await;
        match &r {
            Ok(()) => {
                self.store
                    .set_sync_ok(source_id, VOD_CATS_ENTITY, now_utc())
                    .await?
            }
            Err(e) => {
                self.store
                    .set_sync_error(source_id, VOD_CATS_ENTITY, &e.message)
                    .await?
            }
        }
        r
    }

    fn decode_cat(&self, id: i64) -> (i64, String) {
        self.vod_cats
            .lock()
            .ok()
            .and_then(|g| g.get(&id).cloned())
            .unwrap_or((id / CAT_STRIDE, (id % CAT_STRIDE).to_string()))
    }

    async fn vod_rows(&self, kind: VodKind, a: &VodListArgs) -> Result<Vec<VodRow>> {
        let mut sources = self.caps_sources(Caps::VOD).await?;
        let mut category_ext = None;
        if let Some(cid) = a.category_id {
            let (sid, ext) = self.decode_cat(cid);
            sources.retain(|s| s.id == sid);
            category_ext = Some(ext);
        }
        let limit = a.limit.unwrap_or(60).clamp(1, 500);
        let mut skip = a.offset.unwrap_or(0).max(0);
        let mut remaining = limit;
        let mut out = Vec::new();
        for src in sources {
            let client = self.client(src.id).await?;
            let page = client
                .vod_list(
                    kind,
                    &VodListQuery {
                        category_ext: category_ext.clone(),
                        search: a.query.clone(),
                        offset: skip,
                        limit: remaining,
                    },
                )
                .await?;
            let total = page
                .total
                .unwrap_or(i64::try_from(page.items.len()).unwrap_or(0) + skip);
            let rows = self
                .store
                .upsert_vod_items(src.id, &page.items, now_utc())
                .await?;
            remaining -= i64::try_from(rows.len()).unwrap_or(0);
            skip = (skip - total).max(0);
            out.extend(rows);
            if remaining <= 0 {
                break;
            }
        }
        Ok(out)
    }

    async fn resume_of(&self, r: &VodRow) -> Result<Option<ResumeRow>> {
        self.store.resume(r.source_id, &r.kind, &r.ext_id).await
    }

    fn movie_dto(
        &self,
        r: &VodRow,
        resume: Option<&ResumeRow>,
        poster: Option<String>,
        backdrop: Option<String>,
        compact: bool,
    ) -> MovieDto {
        MovieDto {
            id: r.id,
            title: r.name.clone(),
            year: r.year,
            rating: rating_of(r.rating.as_deref()),
            duration_secs: r.duration_secs,
            poster,
            backdrop,
            plot: if compact { None } else { r.description.clone() },
            genres: if compact {
                Vec::new()
            } else {
                genres_of(r.genre.as_deref())
            },
            category_id: self.category_of(r),
            resume_secs: resume.map(|x| x.position_ms / 1000),
            watched: resume.is_some_and(ResumeRow::finished),
        }
    }

    fn series_dto(
        &self,
        r: &VodRow,
        season_count: i64,
        poster: Option<String>,
        backdrop: Option<String>,
        compact: bool,
    ) -> SeriesDto {
        SeriesDto {
            id: r.id,
            title: r.name.clone(),
            year: r.year,
            rating: rating_of(r.rating.as_deref()),
            poster,
            backdrop,
            plot: if compact { None } else { r.description.clone() },
            genres: if compact {
                Vec::new()
            } else {
                genres_of(r.genre.as_deref())
            },
            category_id: self.category_of(r),
            season_count,
        }
    }

    async fn page_art(&self, rows: &[VodRow], compact: bool) -> HashMap<String, String> {
        let entries: Vec<_> = rows
            .iter()
            .flat_map(|r| {
                [
                    r.art_key.as_deref(),
                    if compact {
                        None
                    } else {
                        r.backdrop_key.as_deref()
                    },
                ]
                .into_iter()
                .flatten()
                .map(move |url| (r.source_id, url))
            })
            .collect();
        match self.images.register_page(&entries).await {
            Ok(art) => art,
            Err(_) => {
                let mut art = HashMap::new();
                for (source_id, url) in entries {
                    if let Some(registered) = self.art(source_id, Some(url)).await {
                        art.insert(url.to_owned(), registered);
                    }
                }
                art
            }
        }
    }

    pub async fn list_movies(&self, a: VodListArgs) -> Result<Vec<MovieDto>> {
        self.movie_page(a, false).await
    }

    pub async fn movie_page(&self, a: VodListArgs, compact: bool) -> Result<Vec<MovieDto>> {
        let rows = self.vod_rows(VodKind::Movie, &a).await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        let (resumes, art) =
            tokio::join!(self.store.vod_resumes(&ids), self.page_art(&rows, compact));
        let resumes = resumes?;
        Ok(rows
            .iter()
            .map(|r| {
                let poster = r.art_key.as_ref().and_then(|url| art.get(url)).cloned();
                let backdrop = (!compact)
                    .then(|| {
                        r.backdrop_key
                            .as_ref()
                            .and_then(|url| art.get(url))
                            .cloned()
                    })
                    .flatten();
                self.movie_dto(r, resumes.get(&r.id), poster, backdrop, compact)
            })
            .collect())
    }

    pub async fn list_series(&self, a: VodListArgs) -> Result<Vec<SeriesDto>> {
        self.series_page(a, false).await
    }

    pub async fn series_page(&self, a: VodListArgs, compact: bool) -> Result<Vec<SeriesDto>> {
        let rows = self.vod_rows(VodKind::Series, &a).await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
        let (counts, art) = tokio::join!(
            self.store.vod_season_counts(&ids),
            self.page_art(&rows, compact)
        );
        let counts = counts?;
        Ok(rows
            .iter()
            .map(|r| {
                let poster = r.art_key.as_ref().and_then(|url| art.get(url)).cloned();
                let backdrop = (!compact)
                    .then(|| {
                        r.backdrop_key
                            .as_ref()
                            .and_then(|url| art.get(url))
                            .cloned()
                    })
                    .flatten();
                self.series_dto(
                    r,
                    counts.get(&r.id).copied().unwrap_or(0),
                    poster,
                    backdrop,
                    compact,
                )
            })
            .collect())
    }

    async fn cached_vod(&self, id: i64, kind: VodKind) -> Result<VodRow> {
        self.store
            .vod_item(id)
            .await?
            .filter(|r| r.kind == kind.as_str())
            .ok_or_else(|| AppError::local("That title is no longer in the library."))
    }

    pub async fn get_movie(&self, id: i64) -> Result<MovieDto> {
        let mut row = self.cached_vod(id, VodKind::Movie).await?;
        // Refresh the details (plot, genres); an unreachable server just means cached data.
        if let Ok(client) = self.client(row.source_id).await
            && let Ok(detail) = client.vod_detail(VodKind::Movie, &row.ext_id).await
            && let Some(r) = self
                .store
                .upsert_vod_items(row.source_id, &[detail], now_utc())
                .await?
                .into_iter()
                .next()
        {
            row = r;
        }
        // Details are lazy on Dispatcharr: description, genre and duration stay empty until
        // `provider-info` has been asked for once. Do that when anything is missing, and cache it.
        if (row.description.is_none() || row.genre.is_none() || row.duration_secs.is_none())
            && let Ok(client) = self.client(row.source_id).await
            && let Ok(Some(mut extra)) = client.vod_enrich(VodKind::Movie, &row.ext_id).await
        {
            extra.title = row.name.clone();
            if let Some(r) = self
                .store
                .upsert_vod_items(row.source_id, &[extra], now_utc())
                .await?
                .into_iter()
                .next()
            {
                row = r;
            }
        }
        let resume = self.resume_of(&row).await?;
        let poster = self.art(row.source_id, row.art_key.as_deref()).await;
        let backdrop = self.art(row.source_id, row.backdrop_key.as_deref()).await;
        let mut dto = self.movie_dto(&row, resume.as_ref(), poster, backdrop, false);
        self.fill_movie(&row, &mut dto).await;
        Ok(dto)
    }

    pub async fn get_series(&self, id: i64) -> Result<SeriesDetailDto> {
        let mut row = self.cached_vod(id, VodKind::Series).await?;
        let client = self.client(row.source_id).await.ok();
        if let Some(c) = &client
            && let Ok(d) = c.vod_detail(VodKind::Series, &row.ext_id).await
            && let Some(r) = self
                .store
                .upsert_vod_items(row.source_id, &[d], now_utc())
                .await?
                .into_iter()
                .next()
        {
            row = r;
        }
        let mut eps = match &client {
            Some(c) => match c.series_episodes(&row.ext_id).await {
                Ok(items) => {
                    self.store
                        .upsert_vod_items(row.source_id, &items, now_utc())
                        .await?
                }
                Err(_) => Vec::new(),
            },
            None => Vec::new(),
        };
        if eps.is_empty()
            && let Some(c) = &client
            && let Ok(Some(mut extra)) = c.vod_enrich(VodKind::Series, &row.ext_id).await
        {
            // The episode list is created by `provider-info`; ask once, then read it again.
            extra.title = row.name.clone();
            if let Some(r) = self
                .store
                .upsert_vod_items(row.source_id, &[extra], now_utc())
                .await?
                .into_iter()
                .next()
            {
                row = r;
            }
            if let Ok(items) = c.series_episodes(&row.ext_id).await {
                eps = self
                    .store
                    .upsert_vod_items(row.source_id, &items, now_utc())
                    .await?;
            }
        }
        if eps.is_empty() {
            eps = self.store.vod_episodes(row.source_id, &row.ext_id).await?; // offline: the cache
        }
        eps.sort_by_key(|e| (e.season.unwrap_or(0), e.episode.unwrap_or(0), e.id));

        let mut seasons: Vec<SeasonDto> = Vec::new();
        let mut resumes: Vec<Option<ResumeRow>> = Vec::new();
        for e in &eps {
            let resume = self.resume_of(e).await?;
            let dto = EpisodeDto {
                id: e.id,
                number: e.episode.unwrap_or(0),
                title: e.name.clone(),
                plot: e.description.clone(),
                duration_secs: e.duration_secs,
                still: self.art(e.source_id, e.art_key.as_deref()).await,
                resume_secs: resume.as_ref().map(|r| r.position_ms / 1000),
                watched: resume.as_ref().is_some_and(ResumeRow::finished),
            };
            let n = e.season.unwrap_or(0);
            match seasons.last_mut() {
                Some(s) if s.number == n => s.episodes.push(dto),
                _ => seasons.push(SeasonDto {
                    number: n,
                    episodes: vec![dto],
                }),
            }
            resumes.push(resume);
        }
        // Continue with the most recently watched episode if unfinished, else the one after it;
        // nothing watched yet starts at the first; everything watched has no next.
        let last = resumes
            .iter()
            .enumerate()
            .filter_map(|(i, r)| r.as_ref().map(|r| (r.updated_at, i)))
            .max();
        let next = match last {
            None => eps.first().map(|e| e.id),
            Some((_, i)) if resumes[i].as_ref().is_some_and(ResumeRow::finished) => {
                eps.get(i + 1).map(|e| e.id)
            }
            Some((_, i)) => Some(eps[i].id),
        };
        let season_count = i64::try_from(
            seasons
                .iter()
                .filter(|s| s.number > 0)
                .count()
                .max(usize::from(!seasons.is_empty())),
        )
        .unwrap_or(0);
        let poster = self.art(row.source_id, row.art_key.as_deref()).await;
        let backdrop = self.art(row.source_id, row.backdrop_key.as_deref()).await;
        let series = self.series_dto(&row, season_count, poster, backdrop, false);
        let mut dto = SeriesDetailDto {
            series,
            seasons,
            next_episode_id: next,
        };
        self.fill_series(&row, &mut dto).await;
        Ok(dto)
    }

    /// Plan for a movie or episode (`/proxy/vod/…` for Dispatcharr, `/movie|series/…` for Xtream).
    pub async fn play_vod(&self, a: PlayVodArgs) -> Result<VodPlay> {
        let kind = match a.kind.as_str() {
            "movie" => VodKind::Movie,
            "episode" => VodKind::Episode,
            _ => return Err(AppError::local("Only movies and episodes can be played.")),
        };
        let row = self.cached_vod(a.id, kind).await?;
        let client = self.client(row.source_id).await?;
        let plan = client.play_vod(&to_item(&row)).await?;
        let resume_secs = if a.resume {
            self.resume_of(&row)
                .await?
                .filter(ResumeRow::resumable)
                .map(|r| r.position_ms / 1000)
        } else {
            None
        };
        let title = match (&row.parent_ext, kind) {
            (Some(p), VodKind::Episode) => {
                let series = self
                    .store
                    .vod_by_ext(row.source_id, "series", p)
                    .await?
                    .map_or_else(String::new, |s| s.name);
                format!("{series} {}", row.name).trim().to_owned()
            }
            _ => row.name.clone(),
        };
        Ok(VodPlay {
            plan,
            resume_secs,
            title,
            kind,
            id: row.id,
            source_id: row.source_id,
            duration_secs: row.duration_secs,
        })
    }

    /// Save the watch position of a movie or episode (call every ~10 s and on pause/stop).
    pub async fn report_vod_position(
        &self,
        id: i64,
        position_secs: i64,
        duration_secs: Option<i64>,
    ) -> Result<()> {
        let row = self
            .store
            .vod_item(id)
            .await?
            .ok_or_else(|| AppError::local("That title is no longer in the library."))?;
        let dur = duration_secs.or(row.duration_secs).map(|d| d * 1000);
        self.store
            .set_resume(
                row.source_id,
                &row.kind,
                &row.ext_id,
                position_secs.max(0) * 1000,
                dur,
                now_utc(),
            )
            .await
    }

    pub(super) async fn continue_item_vod_impl(
        &self,
        r: &ResumeRow,
        progress: Option<f64>,
        remaining: Option<i64>,
    ) -> Result<Option<ContinueItemDto>> {
        match r.kind.as_str() {
            "movie" | "episode" => {
                let Some(row) = self
                    .store
                    .vod_by_ext(r.source_id, &r.kind, &r.item_key)
                    .await?
                else {
                    return Ok(None);
                };
                let subtitle = if row.kind == "episode" {
                    let series = match &row.parent_ext {
                        Some(p) => self
                            .store
                            .vod_by_ext(row.source_id, "series", p)
                            .await?
                            .map(|s| s.name),
                        None => None,
                    };
                    Some(match (series, row.season, row.episode) {
                        (Some(s), Some(se), Some(ep)) => format!("{s} · S{se} E{ep}"),
                        (Some(s), _, _) => s,
                        _ => "Episode".into(),
                    })
                } else {
                    Some("Movie".into())
                };
                Ok(Some(ContinueItemDto {
                    kind: row.kind.clone(),
                    id: format!("{}:{}", row.kind, row.id),
                    channel: None,
                    programme: None,
                    title: row.name.clone(),
                    subtitle,
                    art: self.art(row.source_id, row.art_key.as_deref()).await,
                    progress,
                    remaining_secs: remaining,
                }))
            }
            "recording" => {
                let Some(rec) = self.recording_by_ext(r.source_id, &r.item_key).await? else {
                    return Ok(None);
                };
                let ch = match rec.channel_id {
                    Some(c) => self.get_channel(c).await.ok(),
                    None => None,
                };
                Ok(Some(ContinueItemDto {
                    kind: "recording".into(),
                    id: format!("recording:{}", rec.id),
                    title: rec.title.clone().unwrap_or_else(|| "Recording".into()),
                    subtitle: ch.as_ref().map(|c| c.name.clone()),
                    art: ch.as_ref().and_then(|c| c.logo.clone()),
                    channel: ch,
                    programme: None,
                    progress,
                    remaining_secs: remaining,
                }))
            }
            _ => Ok(None),
        }
    }

    // ---- DVR --------------------------------------------------------------------------------

    async fn recording_by_ext(&self, source_id: i64, ext: &str) -> Result<Option<RecordingRow>> {
        Ok(self
            .store
            .recording_rows(source_id)
            .await?
            .into_iter()
            .find(|r| r.ext_id == ext))
    }

    async fn dvr_client(&self, source_id: i64) -> Result<Arc<dyn SourceClient>> {
        let row = self.store.source(source_id).await?;
        if !Caps::from_db(row.caps).contains(Caps::DVR) {
            return Err(AppError::stream(
                "This server does not allow recording for this account.",
            ));
        }
        self.client(source_id).await
    }

    /// Pull the server's recordings into SQLite (so `isScheduled` and ids are current); an
    /// unreachable server leaves the cached rows in place.
    async fn refresh_recordings(&self, source_id: i64) -> Result<Vec<RecordingRow>> {
        let client = self.client(source_id).await?;
        match client.recordings().await {
            Ok(list) => self.store.sync_recordings(source_id, &list).await,
            Err(e) if e.code == crate::ErrorCode::Network => {
                self.store.recording_rows(source_id).await
            }
            Err(e) => Err(e),
        }
    }

    async fn recording_dto(
        &self,
        r: &RecordingRow,
        rules: &[crate::store::RuleRow],
    ) -> Result<RecordingDto> {
        let channel_name = match r.channel_id {
            Some(c) => self.store.channel(c).await?.map(|c| c.name),
            None => None,
        }
        .unwrap_or_else(|| "Unknown channel".into());
        let title = r.title.clone().unwrap_or_else(|| "Recording".into());
        // Link to the rule that made it: recurring recordings are tagged with the rule id; series
        // recordings carry no tag, so they match by title + tvg_id (`"<tvg>|<title>"`).
        let tvg = r.tvg_id.clone().unwrap_or_default();
        let by_recurring = r.rule_ext.as_deref().and_then(|e| {
            rules
                .iter()
                .find(|x| x.kind == "recurring" && x.ext_id == e)
        });
        let by_series = rules
            .iter()
            .find(|x| x.kind == "series" && x.ext_id == format!("{tvg}|{title}"));
        let linked = by_recurring.or(by_series);
        let rule_id = linked.map(|x| x.id);
        let rule_label = r.rule_label.clone().or_else(|| {
            linked.map(|x| {
                if x.kind == "series" {
                    "Series".to_owned()
                } else {
                    "Recurring".to_owned()
                }
            })
        });
        Ok(RecordingDto {
            id: r.id,
            title,
            subtitle: r.subtitle.clone(),
            description: r.description.clone(),
            season: r.season,
            episode: r.episode,
            channel_id: r.channel_id,
            channel_name,
            start_utc: r.start_utc,
            end_utc: r.end_utc,
            status: r.status().as_str(),
            rule_id,
            rule_label,
            size_bytes: r.size_bytes,
            error: r.error.clone(),
        })
    }

    pub async fn list_recordings(&self) -> Result<RecordingsDto> {
        let mut out = RecordingsDto::default();
        for src in self.caps_sources(Caps::DVR).await? {
            let rows = self.refresh_recordings(src.id).await?;
            let rules = self.store.rule_rows(src.id).await?;
            for r in &rows {
                let dto = self.recording_dto(r, &rules).await?;
                match r.status() {
                    RecStatus::Scheduled => out.upcoming.push(dto),
                    RecStatus::Recording => out.in_progress.push(dto),
                    RecStatus::Completed | RecStatus::Failed => out.completed.push(dto),
                }
            }
        }
        out.upcoming.sort_by_key(|r| r.start_utc);
        out.in_progress.sort_by_key(|r| r.start_utc);
        out.completed
            .sort_by_key(|r| std::cmp::Reverse(r.start_utc));
        Ok(out)
    }

    /// Record a programme (`ProgrammeDto::id`): once, or as a series rule for new episodes.
    pub async fn schedule_recording(&self, programme_id: &str, kind: RecordKindArg) -> Result<()> {
        let (channel_id, start) = parse_programme_id(programme_id)
            .ok_or_else(|| AppError::local("That programme is not valid."))?;
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is no longer in the list."))?;
        let prog = self
            .store
            .programme_row(channel_id, start)
            .await?
            .ok_or_else(|| AppError::local("That programme is no longer in the guide."))?;
        let client = self.dvr_client(ch.source_id).await?;
        let req = RecordRequest {
            channel_ext_id: ch.ext_id.clone(),
            tvg_id: ch.tvg_id.clone(),
            programme_ext_id: prog.ext_id.clone(),
            title: prog.title.clone().unwrap_or_else(|| ch.name.clone()),
            subtitle: prog.subtitle.clone(),
            description: prog.description.clone(),
            season: prog.season,
            episode: prog.episode,
            start_utc: prog.start_utc,
            end_utc: prog.end_utc,
        };
        match kind {
            RecordKindArg::Once => {
                if prog.end_utc <= now_utc() {
                    return Err(AppError::stream("That programme has already finished."));
                }
                client.schedule_recording(&req).await?;
            }
            RecordKindArg::Series => {
                client.schedule_series_rule(&req).await?;
                let rules: Vec<RuleInfo> = client.recording_rules().await.unwrap_or_default();
                self.store.sync_rules(ch.source_id, &rules).await?;
            }
        }
        self.refresh_recordings(ch.source_id).await?;
        self.emit(Event::CatalogUpdated(
            self.counts(ch.source_id, CatalogChange::Recordings).await?,
        ));
        Ok(())
    }

    async fn recording_target(&self, id: i64) -> Result<(RecordingRow, Arc<dyn SourceClient>)> {
        let r = self
            .store
            .recording(id)
            .await?
            .ok_or_else(|| AppError::local("That recording no longer exists."))?;
        let c = self.dvr_client(r.source_id).await?;
        Ok((r, c))
    }

    /// Cancel an upcoming recording.
    pub async fn cancel_recording(&self, id: i64) -> Result<()> {
        let (r, c) = self.recording_target(id).await?;
        if r.status() != RecStatus::Scheduled {
            return Err(AppError::stream(
                "Only upcoming recordings can be cancelled.",
            ));
        }
        c.delete_recording(&r.ext_id).await?;
        self.refresh_recordings(r.source_id).await?;
        Ok(())
    }

    /// Stop a running recording early, keeping what was recorded.
    pub async fn stop_recording(&self, id: i64) -> Result<()> {
        let (r, c) = self.recording_target(id).await?;
        if r.status() != RecStatus::Recording {
            return Err(AppError::stream("That recording is not running."));
        }
        c.stop_recording(&r.ext_id).await?;
        self.refresh_recordings(r.source_id).await?;
        Ok(())
    }

    pub async fn extend_recording(&self, id: i64, minutes: i64) -> Result<()> {
        let (r, c) = self.recording_target(id).await?;
        if r.status() != RecStatus::Recording {
            return Err(AppError::stream(
                "Only a running recording can be extended.",
            ));
        }
        c.extend_recording(&r.ext_id, minutes).await?;
        self.refresh_recordings(r.source_id).await?;
        Ok(())
    }

    /// Delete a finished (or failed) recording and its file.
    pub async fn delete_recording(&self, id: i64) -> Result<()> {
        let (r, c) = self.recording_target(id).await?;
        c.delete_recording(&r.ext_id).await?;
        self.store
            .clear_resume(r.source_id, "recording", &r.ext_id)
            .await?;
        self.refresh_recordings(r.source_id).await?;
        Ok(())
    }

    pub async fn list_recording_rules(&self) -> Result<Vec<RecordingRuleDto>> {
        let mut out = Vec::new();
        for src in self.caps_sources(Caps::DVR).await? {
            let client = self.client(src.id).await?;
            let mut rules = client.recording_rules().await?;
            for r in &mut rules {
                // Name the channel from our own catalogue.
                let ch = match (&r.channel_ext_id, &r.tvg_id) {
                    (Some(e), _) => self.store.channel_by_ext(src.id, e).await?,
                    (None, Some(t)) => {
                        let id = self
                            .store
                            .channels_by_tvg(src.id)
                            .await?
                            .get(t)
                            .and_then(|v| v.first().copied());
                        match id {
                            Some(i) => self.store.channel(i).await?,
                            None => None,
                        }
                    }
                    _ => None,
                };
                r.channel_name = ch.map(|c| c.name).unwrap_or_default();
            }
            for row in self.store.sync_rules(src.id, &rules).await? {
                out.push(RecordingRuleDto {
                    id: row.id,
                    title: row.title,
                    channel_name: row.channel_name,
                    kind: if row.kind == "series" {
                        "series"
                    } else {
                        "recurring"
                    },
                    schedule: row.schedule,
                });
            }
        }
        Ok(out)
    }

    pub async fn delete_recording_rule(&self, id: i64) -> Result<()> {
        let row = self
            .store
            .rule(id)
            .await?
            .ok_or_else(|| AppError::local("That rule no longer exists."))?;
        let client = self.dvr_client(row.source_id).await?;
        let rule = RuleInfo {
            kind: if row.kind == "series" {
                crate::source::RuleKind::Series
            } else {
                crate::source::RuleKind::Recurring
            },
            channel_ext_id: None,
            tvg_id: None,
            ext_id: row.ext_id.clone(),
            title: row.title.clone(),
            channel_name: row.channel_name.clone(),
            schedule: row.schedule.clone(),
        };
        client.delete_rule(&rule).await?;
        self.store
            .sync_rules(
                row.source_id,
                &client.recording_rules().await.unwrap_or_default(),
            )
            .await?;
        Ok(())
    }

    /// Plan for a recording: HLS while it runs, the file when complete. Saved positions resume.
    pub async fn play_recording(&self, id: i64) -> Result<RecordingPlay> {
        let (r, c) = self.recording_target(id).await?;
        if r.status() == RecStatus::Scheduled || r.status() == RecStatus::Failed {
            return Err(AppError::stream(
                "There is nothing to watch for that recording yet.",
            ));
        }
        let info = RecordingInfo {
            ext_id: r.ext_id.clone(),
            channel_ext_id: String::new(),
            title: r.title.clone().unwrap_or_default(),
            start_utc: r.start_utc,
            end_utc: r.end_utc,
            status: r.status(),
            rule_label: r.rule_label.clone(),
            size_bytes: r.size_bytes,
            error: r.error.clone(),
            file_url: r.file_url.clone(),
            rule_ext: r.rule_ext.clone(),
            tvg_id: r.tvg_id.clone(),
            subtitle: r.subtitle.clone(),
            description: r.description.clone(),
            season: r.season,
            episode: r.episode,
            programme_ext: r.programme_ext.clone(),
        };
        let plan = c.recording_plan(&info).await?;
        let resume_secs = self
            .store
            .resume(r.source_id, "recording", &r.ext_id)
            .await?
            .filter(ResumeRow::resumable)
            .map(|x| x.position_ms / 1000);
        Ok(RecordingPlay {
            plan,
            resume_secs,
            title: info.title,
            id: r.id,
            in_progress: r.status() == RecStatus::Recording,
        })
    }

    /// Save the watch position of a recording.
    pub async fn report_recording_position(
        &self,
        id: i64,
        position_secs: i64,
        duration_secs: Option<i64>,
    ) -> Result<()> {
        let r = self
            .store
            .recording(id)
            .await?
            .ok_or_else(|| AppError::local("That recording no longer exists."))?;
        let dur = duration_secs.unwrap_or(r.end_utc - r.start_utc) * 1000;
        self.store
            .set_resume(
                r.source_id,
                "recording",
                &r.ext_id,
                position_secs.max(0) * 1000,
                Some(dur),
                now_utc(),
            )
            .await
    }

    /// Mark programmes that are scheduled or recording (`isScheduled`).
    pub(super) async fn mark_scheduled(&self, progs: &mut [ProgrammeDto]) -> Result<()> {
        let set = self.store.scheduled_programmes().await?;
        if set.is_empty() {
            return Ok(());
        }
        for p in progs {
            p.is_scheduled = set.contains(&(p.channel_id, p.start_utc));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::rating_of;

    #[test]
    fn rating_of_accepts_only_a_real_ten_point_score() {
        assert_eq!(rating_of(Some(" 7.4 ")), Some(7.4));
        assert_eq!(rating_of(Some("10")), Some(10.0));
        for bad in ["0", "0.0", "-1", "11", "n/a", "NaN", "inf", ""] {
            assert_eq!(rating_of(Some(bad)), None, "{bad}");
        }
        assert_eq!(rating_of(None), None);
    }
}
