//! Rich details for one title (guide programme, recording, movie, series, episode): the source's
//! own data first, then TMDB for whatever is still missing. The source always wins.
//!
//! Lookups are cached in `meta_cache` (server programme details for hours, TMDB for days, misses
//! for less) so a details view costs at most a few requests the first time and none afterwards.
//! Nothing here touches a stream endpoint (CLAUDE.md rule 4); the TMDB key lives only in the
//! keychain (rule 7) and every TMDB error is built with it registered for redaction (rule 8).

use std::future::Future;
use std::sync::Arc;

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use super::mediaserver::MediaTargetArg;
use super::{Service, parse_programme_id};
use crate::auth::SecretString;
use crate::error::{AppError, Result};
use crate::images::ImageSize;
use crate::mediaserver::title::title_key;
use crate::source::meta::year_of;
use crate::source::{Credit, ProgrammeExtra, VodExtra, flags};
use crate::store::VodRow;
use crate::sync::now_utc;
use crate::tmdb::{TmdbClient, TmdbEpisode, TmdbKind, TmdbTitle, best_match};

const TMDB_KEY_REF: &str = "metadata:tmdb:key";
const TMDB_CFG: &str = "metadata.tmdb";
const TMDB_LANGUAGE: &str = "en-US";

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;
/// Server programme details: they change when the guide refreshes.
const SOURCE_TTL: (i64, i64) = (6 * HOUR, HOUR);
/// TMDB titles, episodes and seasons.
const TMDB_TTL: (i64, i64) = (7 * DAY, 2 * DAY);
/// Title searches (which TMDB id a title is): stable, but retry misses now and then.
const SEARCH_TTL: (i64, i64) = (30 * DAY, 3 * DAY);
/// The artwork picked for a programme title (misses are retried sooner: a TMDB key may be added).
const ART_TTL: (i64, i64) = (30 * DAY, DAY);
/// Anything older than this is pruned at startup.
const PRUNE_AFTER: i64 = 60 * DAY;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct TmdbConfig {
    language: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TmdbStatusDto {
    /// A key is saved.
    pub configured: bool,
    pub language: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PersonDto {
    pub name: String,
    /// Character (cast) or job.
    pub role: Option<String>,
    /// `dlimg://` headshot.
    pub photo: Option<String>,
    pub guest: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CrewDto {
    /// `director` | `writer` | `creator` | `producer` | `presenter`.
    pub job: &'static str,
    pub names: Vec<String>,
}

/// Everything a details view shows. Source data wins; TMDB fills the gaps (`fromTmdb` says it
/// contributed).
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDetailsDto {
    /// `programme` | `recording` | `movie` | `series` | `episode`.
    pub kind: &'static str,
    /// The show's or movie's title.
    pub title: String,
    pub episode_title: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    /// This airing / episode / movie.
    pub description: Option<String>,
    /// The show's own overview, when it differs from `description`.
    pub overview: Option<String>,
    pub tagline: Option<String>,
    pub genres: Vec<String>,
    pub year: Option<i64>,
    /// Original air date or release date, `YYYY-MM-DD`.
    pub air_date: Option<String>,
    pub runtime_mins: Option<i64>,
    pub certification: Option<String>,
    /// 0..10.
    pub score: Option<f64>,
    pub country: Option<String>,
    pub language: Option<String>,
    pub network: Option<String>,
    /// `Returning Series`, `Ended`…
    pub status: Option<String>,
    pub season_count: Option<i64>,
    pub cast: Vec<PersonDto>,
    pub crew: Vec<CrewDto>,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub still: Option<String>,
    /// Transparent title logo.
    pub logo: Option<String>,
    pub video_quality: Option<String>,
    pub aspect_ratio: Option<String>,
    pub audio: Option<String>,
    pub is_new: bool,
    pub is_live: bool,
    pub is_premiere: bool,
    pub is_finale: bool,
    /// Shown before (a rerun).
    pub is_repeat: bool,
    pub advisories: Vec<String>,
    /// `https://www.youtube.com/watch?v=…`
    pub trailer_url: Option<String>,
    /// The title's TMDB page (attribution link).
    pub tmdb_url: Option<String>,
    pub from_tmdb: bool,
}

/// What we ask TMDB about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hint {
    Movie,
    Tv,
    /// Live guide rows with no episode numbers: try a show, then a movie.
    Unknown,
    /// News, sports, live events: TMDB has nothing useful.
    Skip,
}

#[derive(Debug, Clone)]
struct Lookup {
    hint: Hint,
    title: String,
    year: Option<i64>,
    tmdb_id: Option<i64>,
    imdb_id: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
}

#[derive(Debug, Default)]
struct Found {
    title: Option<TmdbTitle>,
    episode: Option<TmdbEpisode>,
}

/// Categories that say "TMDB has nothing for this".
fn skip_categories(cats: &[String]) -> bool {
    const SKIP: &[&str] = &[
        "news",
        "sports",
        "sport",
        "sports event",
        "sports talk",
        "weather",
        "paid programming",
        "shopping",
        "auto racing",
        "football",
        "soccer",
        "basketball",
        "baseball",
        "hockey",
        "golf",
        "tennis",
        "boxing",
        "wrestling",
        "religious",
    ];
    cats.iter()
        .any(|c| SKIP.contains(&c.trim().to_ascii_lowercase().as_str()))
}

fn movie_categories(cats: &[String]) -> bool {
    cats.iter().any(|c| {
        let c = c.trim().to_ascii_lowercase();
        c == "movie" || c == "movies" || c == "film" || c == "feature film"
    })
}

/// Episode names that carry no information (`Episode 3`, the show's own name, `S01E03`).
fn generic_episode_title(name: &str, show: &str) -> bool {
    let n = name.trim();
    if n.is_empty() || title_key(n) == title_key(show) {
        return true;
    }
    let lower = n.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("episode")
        .or_else(|| lower.strip_prefix("ep."))
        .or_else(|| lower.strip_prefix("ep"))
        .map(str::trim);
    if rest.is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit())) {
        return true;
    }
    // `S01E03` alone, or `Show - S01E03` with nothing after.
    let b = lower.as_bytes();
    let tail = lower.rfind('s').map(|i| &b[i..]).unwrap_or_default();
    tail.len() >= 4
        && tail.len() <= 8
        && tail[1].is_ascii_digit()
        && tail.contains(&b'e')
        && tail[1..].iter().all(|c| c.is_ascii_digit() || *c == b'e')
}

/// The source's name if it says something, else TMDB's.
fn pick_text(source: Option<String>, tmdb: Option<String>) -> Option<String> {
    source
        .filter(|s| !s.trim().is_empty())
        .or_else(|| tmdb.filter(|s| !s.trim().is_empty()))
}

fn non_empty<T>(source: Vec<T>, fallback: Vec<T>) -> Vec<T> {
    if source.is_empty() { fallback } else { source }
}

fn rating_of(r: Option<&str>) -> Option<f64> {
    let v: f64 = r?.trim().parse().ok()?;
    (v > 0.0 && v <= 10.0).then_some(v)
}

fn genres_of(g: Option<&str>) -> Vec<String> {
    g.map(|g| {
        g.split([',', '/', '|'])
            .map(|x| x.trim().to_owned())
            .filter(|x| !x.is_empty())
            .collect()
    })
    .unwrap_or_default()
}

fn trailer_url(id: Option<&str>) -> Option<String> {
    let id = id?;
    (id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    .then(|| format!("https://www.youtube.com/watch?v={id}"))
}

impl Service {
    // ---- TMDB settings --------------------------------------------------------------------

    async fn tmdb_config(&self) -> TmdbConfig {
        match self.store.kv_get(TMDB_CFG).await {
            Ok(Some((v, _))) => serde_json::from_str(&v).unwrap_or_default(),
            _ => TmdbConfig::default(),
        }
    }

    async fn tmdb_language(&self) -> String {
        self.tmdb_config()
            .await
            .language
            .unwrap_or_else(|| TMDB_LANGUAGE.to_owned())
    }

    fn tmdb_base(&self) -> String {
        self.tmdb_api
            .read()
            .map(|u| u.clone())
            .unwrap_or_else(|_| crate::tmdb::TMDB_API.to_owned())
    }

    /// Point TMDB calls elsewhere (tests use a mock server).
    #[doc(hidden)]
    pub fn set_tmdb_api(&self, url: &str) {
        if let Ok(mut u) = self.tmdb_api.write() {
            url.clone_into(&mut u);
        }
    }

    /// The TMDB client, when a key is saved.
    async fn tmdb(&self) -> Option<Arc<TmdbClient>> {
        let mut g = self.tmdb.lock().await;
        if let Some(slot) = g.as_ref() {
            return slot.clone();
        }
        let built = match self.secrets.get(TMDB_KEY_REF) {
            Ok(Some(key)) => TmdbClient::new(&self.tmdb_base(), key, &self.tmdb_language().await)
                .ok()
                .map(Arc::new),
            // A keychain hiccup is not "no key": ask again next time.
            Err(_) => return None,
            Ok(None) => None,
        };
        *g = Some(built.clone());
        built
    }

    pub async fn tmdb_status(&self) -> Result<TmdbStatusDto> {
        Ok(TmdbStatusDto {
            configured: self.secrets.get(TMDB_KEY_REF)?.is_some(),
            language: self.tmdb_language().await,
        })
    }

    /// Check the key with TMDB, then keep it in the keychain. A rejected key is not saved.
    pub async fn tmdb_set_key(&self, key: SecretString) -> Result<TmdbStatusDto> {
        let trimmed = SecretString::new(key.expose().trim());
        let client = TmdbClient::new(
            &self.tmdb_base(),
            trimmed.clone(),
            &self.tmdb_language().await,
        )?;
        client.check().await?;
        self.secrets.set(TMDB_KEY_REF, &trimmed)?;
        *self.tmdb.lock().await = Some(Some(Arc::new(client)));
        // Misses cached without a key (or with another account) should be asked again.
        self.store.meta_clear_prefix("tmdb:").await?;
        self.tmdb_status().await
    }

    /// Forget the TMDB key (it leaves the keychain) and what it fetched.
    pub async fn tmdb_clear_key(&self) -> Result<TmdbStatusDto> {
        self.secrets.delete(TMDB_KEY_REF)?;
        *self.tmdb.lock().await = Some(None);
        self.store.meta_clear_prefix("tmdb:").await?;
        self.tmdb_status().await
    }

    /// Startup housekeeping for `meta_cache`.
    pub async fn prune_metadata(&self) -> Result<u64> {
        self.store.meta_prune(now_utc() - PRUNE_AFTER).await
    }

    // ---- cache ------------------------------------------------------------------------------

    /// Cached `Option<T>` under `key`: fresh hits within `ttl.0`, misses within `ttl.1`. A failed
    /// fetch falls back to a stale entry, then to the error.
    async fn cached<T, F>(&self, key: &str, ttl: (i64, i64), fetch: F) -> Result<Option<T>>
    where
        T: Serialize + DeserializeOwned,
        F: Future<Output = Result<Option<T>>>,
    {
        let now = now_utc();
        let stale: Option<Option<T>> = match self.store.meta_get(key).await? {
            Some((body, at)) => match serde_json::from_str::<Option<T>>(&body) {
                Ok(v) => {
                    let max = if v.is_some() { ttl.0 } else { ttl.1 };
                    if now - at < max {
                        return Ok(v);
                    }
                    Some(v)
                }
                Err(_) => None,
            },
            None => None,
        };
        match fetch.await {
            Ok(v) => {
                self.store
                    .meta_put(key, &serde_json::to_string(&v)?, now)
                    .await?;
                Ok(v)
            }
            Err(e) => match stale {
                Some(v) => Ok(v),
                None => Err(e),
            },
        }
    }

    async fn programme_extra(
        &self,
        source_id: i64,
        ext_id: Option<&str>,
    ) -> Option<ProgrammeExtra> {
        let ext = ext_id.filter(|e| !e.is_empty())?;
        let client = self.client(source_id).await.ok()?;
        self.cached(&format!("dpg:{source_id}:{ext}"), SOURCE_TTL, async {
            client.programme_detail(ext).await
        })
        .await
        .ok()
        .flatten()
    }

    // ---- TMDB lookups -----------------------------------------------------------------------

    async fn tmdb_search(
        &self,
        c: &TmdbClient,
        kind: TmdbKind,
        title: &str,
        year: Option<i64>,
    ) -> Option<i64> {
        let key = title_key(&crate::mediaserver::title::strip_decoration(title).0);
        if key.is_empty() {
            return None;
        }
        let strict = kind == TmdbKind::Movie;
        let lang = c.language().to_owned();
        let cache = format!(
            "tmdb:{lang}:search:{}:{key}:{}",
            kind.as_str(),
            year.map(|y| y.to_string()).unwrap_or_default()
        );
        self.cached(&cache, SEARCH_TTL, async {
            let (clean, title_year) = crate::mediaserver::title::strip_decoration(title);
            // Search with the year first (sharper), then without: guide years are often the
            // episode's, not the show's first air date.
            let y = year.or(title_year);
            let mut hits = c
                .search(kind, &clean, if strict { y } else { None })
                .await?;
            if hits.is_empty() && strict && y.is_some() {
                hits = c.search(kind, &clean, None).await?;
            }
            Ok(best_match(&hits, title, y, strict))
        })
        .await
        .ok()
        .flatten()
    }

    async fn tmdb_title(&self, c: &TmdbClient, kind: TmdbKind, id: i64) -> Option<TmdbTitle> {
        let key = format!("tmdb:{}:title:{}:{id}", c.language(), kind.as_str());
        self.cached(&key, TMDB_TTL, c.title(kind, id))
            .await
            .ok()
            .flatten()
    }

    async fn tmdb_episode(&self, c: &TmdbClient, tv: i64, s: i64, e: i64) -> Option<TmdbEpisode> {
        let key = format!("tmdb:{}:ep:{tv}:{s}:{e}", c.language());
        self.cached(&key, TMDB_TTL, c.episode(tv, s, e))
            .await
            .ok()
            .flatten()
    }

    async fn tmdb_season(&self, c: &TmdbClient, tv: i64, s: i64) -> Vec<TmdbEpisode> {
        let key = format!("tmdb:{}:season:{tv}:{s}", c.language());
        self.cached(&key, TMDB_TTL, async {
            let eps = c.season(tv, s).await?;
            Ok((!eps.is_empty()).then_some(eps))
        })
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
    }

    /// Which TMDB title a lookup names: the source's TMDB id, then its IMDb id, then a search.
    async fn tmdb_resolve(&self, c: &TmdbClient, l: &Lookup) -> Option<(TmdbKind, i64)> {
        let kinds: &[TmdbKind] = match l.hint {
            Hint::Movie => &[TmdbKind::Movie],
            Hint::Tv => &[TmdbKind::Tv],
            Hint::Unknown => &[TmdbKind::Tv, TmdbKind::Movie],
            Hint::Skip => &[],
        };
        if let Some(id) = l.tmdb_id
            && let Some(k) = kinds.first()
        {
            return Some((*k, id));
        }
        if let Some(imdb) = l.imdb_id.as_deref() {
            let key = format!("tmdb:find:{imdb}");
            let found: Option<(TmdbKind, i64)> = self
                .cached(&key, SEARCH_TTL, c.find_imdb(imdb))
                .await
                .ok()
                .flatten();
            if let Some(f) = found {
                return Some(f);
            }
        }
        for k in kinds {
            if let Some(id) = self.tmdb_search(c, *k, &l.title, l.year).await {
                return Some((*k, id));
            }
        }
        None
    }

    async fn tmdb_lookup(&self, l: &Lookup) -> Found {
        if l.hint == Hint::Skip && l.tmdb_id.is_none() {
            return Found::default();
        }
        let Some(c) = self.tmdb().await else {
            return Found::default();
        };
        let Some((kind, id)) = self.tmdb_resolve(&c, l).await else {
            return Found::default();
        };
        let title = self.tmdb_title(&c, kind, id).await;
        let episode = match (kind, l.season, l.episode) {
            (TmdbKind::Tv, Some(s), Some(e)) => self.tmdb_episode(&c, id, s, e).await,
            _ => None,
        };
        Found { title, episode }
    }

    // ---- images -----------------------------------------------------------------------------

    async fn reg(&self, source_id: i64, url: Option<&str>, size: ImageSize) -> Option<String> {
        self.images.register(source_id, url?, size).await.ok()
    }

    async fn people(&self, source_id: i64, list: &[Credit], tmdb: &[Credit]) -> Vec<PersonDto> {
        let mut out = Vec::with_capacity(list.len());
        for p in list.iter().take(24) {
            // The source names people; TMDB adds their photo when it lists the same name.
            let photo_url = p.photo.clone().or_else(|| {
                let k = title_key(&p.name);
                tmdb.iter()
                    .find(|t| title_key(&t.name) == k)
                    .and_then(|t| t.photo.clone())
            });
            out.push(PersonDto {
                name: p.name.clone(),
                role: p.role.clone(),
                photo: self
                    .reg(source_id, photo_url.as_deref(), ImageSize::Grid)
                    .await,
                guest: p.guest,
            });
        }
        out
    }

    // ---- details ----------------------------------------------------------------------------

    /// Details for a guide programme, a recording or a VOD title. Never touches a stream.
    pub async fn media_details(&self, target: MediaTargetArg) -> Result<MediaDetailsDto> {
        match target {
            MediaTargetArg::Programme { id } => self.programme_details(&id).await,
            MediaTargetArg::Recording { id } => self.recording_details(id).await,
            MediaTargetArg::Vod { id } => self.vod_details(id).await,
        }
    }

    async fn programme_details(&self, id: &str) -> Result<MediaDetailsDto> {
        let (channel_id, start) = parse_programme_id(id)
            .ok_or_else(|| AppError::local("That programme is not valid."))?;
        let p = self
            .store
            .programme_row(channel_id, start)
            .await?
            .ok_or_else(|| AppError::local("That programme is no longer in the guide."))?;
        let ch = self
            .store
            .channel(channel_id)
            .await?
            .ok_or_else(|| AppError::local("That channel is gone."))?;
        let airing = Airing {
            source_id: ch.source_id,
            kind: "programme",
            title: p.title.clone().unwrap_or_else(|| ch.name.clone()),
            subtitle: p.subtitle.clone(),
            description: p.description.clone(),
            season: p.season,
            episode: p.episode,
            flags: p.flags,
            ext_id: p.ext_id.clone(),
            runtime_mins: Some((p.end_utc - p.start_utc) / 60).filter(|m| *m > 0),
        };
        Ok(self.airing_details(airing).await)
    }

    /// Artwork for a guide programme, as a `dlimg://` URL: the guide's own icon first, then the
    /// server's programme details, then TMDB (backdrop before poster: heroes and cards are
    /// landscape). The chosen source URL is cached per title, so later views (and other airings
    /// of the same episode) need no lookups; the image itself is kept by the image cache.
    /// Never touches a stream endpoint (CLAUDE.md rule 4).
    pub async fn programme_art(&self, id: &str) -> Result<Option<String>> {
        let (channel_id, start) = parse_programme_id(id)
            .ok_or_else(|| AppError::local("That programme is not valid."))?;
        let Some(p) = self.store.programme_row(channel_id, start).await? else {
            return Ok(None);
        };
        let Some(ch) = self.store.channel(channel_id).await? else {
            return Ok(None);
        };
        let src = match p.icon.clone() {
            Some(icon) => Some(icon),
            None => {
                let title = p.title.clone().unwrap_or_else(|| ch.name.clone());
                let key = format!(
                    "part:{}:{}:{}:{}:{}",
                    ch.source_id,
                    p.ext_id.as_deref().unwrap_or_default(),
                    title_key(&title),
                    p.season.map(|n| n.to_string()).unwrap_or_default(),
                    p.episode.map(|n| n.to_string()).unwrap_or_default(),
                );
                let airing = Airing {
                    source_id: ch.source_id,
                    kind: "programme",
                    title,
                    subtitle: p.subtitle.clone(),
                    description: None,
                    season: p.season,
                    episode: p.episode,
                    flags: p.flags,
                    ext_id: p.ext_id.clone(),
                    runtime_mins: None,
                };
                self.cached(&key, ART_TTL, async { Ok(self.airing_art(&airing).await) })
                    .await
                    .ok()
                    .flatten()
            }
        };
        Ok(self
            .reg(ch.source_id, src.as_deref(), ImageSize::Detail)
            .await)
    }

    /// Source artwork URL for an airing without a guide icon: server details, then TMDB.
    async fn airing_art(&self, a: &Airing) -> Option<String> {
        let x = self
            .programme_extra(a.source_id, a.ext_id.as_deref())
            .await
            .unwrap_or_default();
        if let Some(u) = x.images.first() {
            return Some(u.clone());
        }
        let found = self.tmdb_lookup(&airing_lookup(a, &x)).await;
        found.episode.and_then(|e| e.still).or_else(|| {
            let t = found.title?;
            t.backdrop.or(t.poster)
        })
    }

    async fn recording_details(&self, id: i64) -> Result<MediaDetailsDto> {
        let r = self
            .store
            .recording(id)
            .await?
            .ok_or_else(|| AppError::local("That recording is gone."))?;
        // The guide row (if still there) has flags and is the freshest copy of the text.
        let row = match r.channel_id {
            Some(c) => self.store.programme_row(c, r.start_utc).await?,
            None => None,
        };
        let airing = Airing {
            source_id: r.source_id,
            kind: "recording",
            title: r.title.clone().unwrap_or_else(|| "Recording".into()),
            subtitle: r
                .subtitle
                .clone()
                .or_else(|| row.as_ref().and_then(|p| p.subtitle.clone())),
            description: r
                .description
                .clone()
                .or_else(|| row.as_ref().and_then(|p| p.description.clone())),
            season: r.season.or_else(|| row.as_ref().and_then(|p| p.season)),
            episode: r.episode.or_else(|| row.as_ref().and_then(|p| p.episode)),
            flags: row.as_ref().map_or(0, |p| p.flags),
            ext_id: r
                .programme_ext
                .clone()
                .or_else(|| row.as_ref().and_then(|p| p.ext_id.clone())),
            runtime_mins: Some((r.end_utc - r.start_utc) / 60).filter(|m| *m > 0),
        };
        Ok(self.airing_details(airing).await)
    }

    async fn airing_details(&self, a: Airing) -> MediaDetailsDto {
        let x = self
            .programme_extra(a.source_id, a.ext_id.as_deref())
            .await
            .unwrap_or_default();
        let f = self.tmdb_lookup(&airing_lookup(&a, &x)).await;
        let t = f.title.clone().unwrap_or_default();
        let ep = f.episode.clone().unwrap_or_default();
        let is_movie = t.kind == Some(TmdbKind::Movie);

        let description = pick_text(
            a.description.clone(),
            if is_movie {
                t.overview.clone()
            } else {
                ep.overview.clone()
            },
        );
        let overview = (!is_movie)
            .then(|| t.overview.clone())
            .flatten()
            .filter(|o| Some(o) != description.as_ref());
        let episode_title = pick_text(
            a.subtitle
                .clone()
                .filter(|s| !generic_episode_title(s, &a.title)),
            ep.name.clone(),
        )
        .or(a.subtitle.clone());

        // Cast: the guide's credits win; TMDB adds photos, or the whole list when the guide has
        // none (the episode's guest stars first, then the show's regulars).
        let tmdb_people: Vec<Credit> = ep
            .guest_stars
            .iter()
            .map(|g| Credit {
                guest: true,
                ..g.clone()
            })
            .chain(t.cast.iter().cloned())
            .collect();
        let cast_src = non_empty(x.actors.clone(), tmdb_people.clone());
        let cast = self.people(a.source_id, &cast_src, &tmdb_people).await;

        let mut crew = Vec::new();
        push_crew(&mut crew, "creator", t.creators.clone());
        push_crew(
            &mut crew,
            "director",
            non_empty(
                x.directors.clone(),
                non_empty(ep.directors.clone(), t.directors.clone()),
            ),
        );
        push_crew(
            &mut crew,
            "writer",
            non_empty(
                x.writers.clone(),
                non_empty(ep.writers.clone(), t.writers.clone()),
            ),
        );
        push_crew(&mut crew, "presenter", x.presenters.clone());
        push_crew(&mut crew, "producer", x.producers.clone());

        let poster_src = x.images.first().cloned().or(t.poster.clone());
        MediaDetailsDto {
            kind: a.kind,
            title: a.title.clone(),
            episode_title,
            season: a.season.or((ep.season > 0).then_some(ep.season)),
            episode: a.episode.or((ep.episode > 0).then_some(ep.episode)),
            description,
            overview,
            tagline: t.tagline.clone(),
            genres: non_empty(x.categories.clone(), t.genres.clone()),
            year: x.production_date.as_deref().and_then(year_of).or(t.year),
            air_date: x
                .original_air_date
                .clone()
                .or(ep.air_date.clone())
                .or_else(|| is_movie.then(|| t.release_date.clone()).flatten()),
            runtime_mins: x
                .runtime_mins
                .or(ep.runtime_mins)
                .or(if is_movie { t.runtime_mins } else { None })
                .or(a.runtime_mins),
            certification: x.rating.clone().or(t.certification.clone()),
            score: x.score.or(ep.score).or(t.score),
            country: x.country.clone().or_else(|| t.countries.first().cloned()),
            language: x.language.clone(),
            network: t.networks.first().cloned(),
            status: t.status.clone().filter(|_| !is_movie),
            season_count: t.season_count,
            cast,
            crew,
            poster: self
                .reg(a.source_id, poster_src.as_deref(), ImageSize::Detail)
                .await,
            backdrop: self
                .reg(a.source_id, t.backdrop.as_deref(), ImageSize::Detail)
                .await,
            still: self
                .reg(a.source_id, ep.still.as_deref(), ImageSize::Detail)
                .await,
            logo: self
                .reg(a.source_id, t.logo.as_deref(), ImageSize::Detail)
                .await,
            video_quality: x.video_quality.clone(),
            aspect_ratio: x.aspect_ratio.clone(),
            audio: x.stereo.clone(),
            is_new: a.flags & flags::NEW != 0,
            is_live: a.flags & flags::LIVE != 0,
            is_premiere: a.flags & flags::PREMIERE != 0,
            is_finale: a.flags & flags::FINALE != 0,
            is_repeat: x.previously_shown && a.flags & (flags::NEW | flags::PREMIERE) == 0,
            advisories: x.content_advisory.clone(),
            trailer_url: trailer_url(t.trailer.as_deref()),
            tmdb_url: f.title.as_ref().map(tmdb_page),
            from_tmdb: f.title.is_some() || f.episode.is_some(),
        }
    }

    async fn vod_details(&self, id: i64) -> Result<MediaDetailsDto> {
        let row = self
            .store
            .vod_item(id)
            .await?
            .ok_or_else(|| AppError::local("That title is gone."))?;
        let series = if row.kind == "episode" {
            match row.parent_ext.as_deref() {
                Some(p) => self.store.vod_by_ext(row.source_id, "series", p).await?,
                None => None,
            }
        } else {
            None
        };
        let show = series.as_ref().unwrap_or(&row);
        let hint = if row.kind == "movie" {
            Hint::Movie
        } else {
            Hint::Tv
        };
        let lookup = Lookup {
            hint,
            title: show.name.clone(),
            year: show.year,
            tmdb_id: show.tmdb_id.as_deref().and_then(|t| t.parse().ok()),
            imdb_id: show.imdb_id.clone(),
            season: (row.kind == "episode").then_some(row.season).flatten(),
            episode: (row.kind == "episode").then_some(row.episode).flatten(),
        };
        let f = self.tmdb_lookup(&lookup).await;
        Ok(self.vod_merge(&row, series.as_ref(), &f).await)
    }

    async fn vod_merge(&self, row: &VodRow, series: Option<&VodRow>, f: &Found) -> MediaDetailsDto {
        let t = f.title.clone().unwrap_or_default();
        let ep = f.episode.clone().unwrap_or_default();
        let show = series.unwrap_or(row);
        let x: VodExtra = show.extra();
        let is_episode = row.kind == "episode";
        let sid = row.source_id;

        let tmdb_people: Vec<Credit> = ep
            .guest_stars
            .iter()
            .map(|g| Credit {
                guest: true,
                ..g.clone()
            })
            .chain(t.cast.iter().cloned())
            .collect();
        let cast_src: Vec<Credit> = if x.cast.is_empty() {
            tmdb_people.clone()
        } else {
            x.cast
                .iter()
                .map(|n| Credit {
                    name: n.clone(),
                    ..Credit::default()
                })
                .collect()
        };
        let cast = self.people(sid, &cast_src, &tmdb_people).await;
        let mut crew = Vec::new();
        push_crew(&mut crew, "creator", t.creators.clone());
        push_crew(
            &mut crew,
            "director",
            non_empty(
                x.directors.clone(),
                non_empty(ep.directors.clone(), t.directors.clone()),
            ),
        );
        push_crew(
            &mut crew,
            "writer",
            non_empty(ep.writers.clone(), t.writers.clone()),
        );

        let description = if is_episode {
            pick_text(row.description.clone(), ep.overview.clone())
        } else {
            pick_text(row.description.clone(), t.overview.clone())
        };
        let overview = if is_episode {
            pick_text(show.description.clone(), t.overview.clone())
                .filter(|o| Some(o) != description.as_ref())
        } else {
            None
        };
        let poster_src = show.art_key.clone().or(t.poster.clone());
        let backdrop_src = show.backdrop_key.clone().or(t.backdrop.clone());
        let still_src = if is_episode {
            row.art_key.clone().or(ep.still.clone())
        } else {
            None
        };
        MediaDetailsDto {
            kind: match row.kind.as_str() {
                "movie" => "movie",
                "series" => "series",
                _ => "episode",
            },
            title: show.name.clone(),
            episode_title: if is_episode {
                pick_text(
                    Some(row.name.clone()).filter(|n| !generic_episode_title(n, &show.name)),
                    ep.name.clone(),
                )
                .or(Some(row.name.clone()))
            } else {
                None
            },
            season: if is_episode { row.season } else { None },
            episode: if is_episode { row.episode } else { None },
            description,
            overview,
            tagline: t.tagline.clone(),
            genres: non_empty(genres_of(show.genre.as_deref()), t.genres.clone()),
            year: show.year.or(t.year),
            air_date: if is_episode {
                ep.air_date.clone()
            } else {
                x.release_date.clone().or(t.release_date.clone())
            },
            runtime_mins: row
                .duration_secs
                .map(|d| d / 60)
                .filter(|m| *m > 0)
                .or(if is_episode {
                    ep.runtime_mins
                } else {
                    t.runtime_mins
                }),
            certification: x.certification.clone().or(t.certification.clone()),
            score: if is_episode {
                rating_of(row.rating.as_deref()).or(ep.score)
            } else {
                rating_of(show.rating.as_deref()).or(t.score)
            },
            country: x.country.clone().or_else(|| t.countries.first().cloned()),
            language: None,
            network: t.networks.first().cloned(),
            status: t.status.clone().filter(|_| row.kind != "movie"),
            season_count: t.season_count,
            cast,
            crew,
            poster: self
                .reg(sid, poster_src.as_deref(), ImageSize::Detail)
                .await,
            backdrop: self
                .reg(sid, backdrop_src.as_deref(), ImageSize::Detail)
                .await,
            still: self.reg(sid, still_src.as_deref(), ImageSize::Detail).await,
            logo: self.reg(sid, t.logo.as_deref(), ImageSize::Detail).await,
            trailer_url: trailer_url(x.trailer.as_deref().or(t.trailer.as_deref())),
            tmdb_url: f.title.as_ref().map(tmdb_page),
            from_tmdb: f.title.is_some() || f.episode.is_some(),
            ..MediaDetailsDto::default()
        }
    }

    // ---- filling VOD lists ------------------------------------------------------------------

    /// TMDB data for a VOD movie or series row, when a key is saved and the title is found.
    pub(super) async fn vod_tmdb(&self, row: &VodRow) -> Option<TmdbTitle> {
        let lookup = Lookup {
            hint: if row.kind == "movie" {
                Hint::Movie
            } else {
                Hint::Tv
            },
            title: row.name.clone(),
            year: row.year,
            tmdb_id: row.tmdb_id.as_deref().and_then(|t| t.parse().ok()),
            imdb_id: row.imdb_id.clone(),
            season: None,
            episode: None,
        };
        self.tmdb_lookup(&lookup).await.title
    }

    /// TMDB's episodes for the given seasons of a show (cached per season).
    pub(super) async fn tmdb_seasons(&self, tv: i64, seasons: &[i64]) -> Vec<TmdbEpisode> {
        let Some(c) = self.tmdb().await else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for s in seasons.iter().take(40) {
            out.extend(self.tmdb_season(&c, tv, *s).await);
        }
        out
    }

    /// Fill what a movie's own data leaves empty (plot, art, genres, runtime, score) from TMDB.
    pub(super) async fn fill_movie(&self, row: &VodRow, d: &mut super::MovieDto) {
        if d.plot.is_some()
            && d.poster.is_some()
            && d.backdrop.is_some()
            && !d.genres.is_empty()
            && d.duration_secs.is_some()
            && d.year.is_some()
        {
            return;
        }
        let Some(t) = self.vod_tmdb(row).await else {
            return;
        };
        let sid = row.source_id;
        if d.plot.is_none() {
            d.plot.clone_from(&t.overview);
        }
        if d.poster.is_none() {
            d.poster = self.reg(sid, t.poster.as_deref(), ImageSize::Detail).await;
        }
        if d.backdrop.is_none() {
            d.backdrop = self
                .reg(sid, t.backdrop.as_deref(), ImageSize::Detail)
                .await;
        }
        if d.genres.is_empty() {
            d.genres.clone_from(&t.genres);
        }
        if d.duration_secs.is_none() {
            d.duration_secs = t.runtime_mins.map(|m| m * 60);
        }
        d.year = d.year.or(t.year);
        d.rating = d.rating.or(t.score);
    }

    /// Same for a series and its episodes (titles, plots and stills per season).
    pub(super) async fn fill_series(&self, row: &VodRow, d: &mut super::SeriesDetailDto) {
        let s = &d.series;
        let eps_complete = d.seasons.iter().flat_map(|x| &x.episodes).all(|e| {
            e.plot.is_some() && e.still.is_some() && !generic_episode_title(&e.title, &s.title)
        });
        if s.plot.is_some()
            && s.poster.is_some()
            && s.backdrop.is_some()
            && !s.genres.is_empty()
            && eps_complete
        {
            return;
        }
        let Some(t) = self.vod_tmdb(row).await else {
            return;
        };
        let sid = row.source_id;
        let s = &mut d.series;
        if s.plot.is_none() {
            s.plot.clone_from(&t.overview);
        }
        if s.poster.is_none() {
            s.poster = self.reg(sid, t.poster.as_deref(), ImageSize::Detail).await;
        }
        if s.backdrop.is_none() {
            s.backdrop = self
                .reg(sid, t.backdrop.as_deref(), ImageSize::Detail)
                .await;
        }
        if s.genres.is_empty() {
            s.genres.clone_from(&t.genres);
        }
        s.year = s.year.or(t.year);
        s.rating = s.rating.or(t.score);
        if eps_complete || t.kind != Some(TmdbKind::Tv) {
            return;
        }
        let numbers: Vec<i64> = d
            .seasons
            .iter()
            .map(|x| x.number)
            .filter(|n| *n > 0)
            .collect();
        let tm = self.tmdb_seasons(t.id, &numbers).await;
        let show = d.series.title.clone();
        for season in &mut d.seasons {
            for e in &mut season.episodes {
                let Some(m) = tm
                    .iter()
                    .find(|m| m.season == season.number && m.episode == e.number)
                else {
                    continue;
                };
                if generic_episode_title(&e.title, &show)
                    && let Some(n) = m.name.clone()
                {
                    e.title = n;
                }
                if e.plot.is_none() {
                    e.plot.clone_from(&m.overview);
                }
                if e.still.is_none() {
                    e.still = self.reg(sid, m.still.as_deref(), ImageSize::Detail).await;
                }
                if e.duration_secs.is_none() {
                    e.duration_secs = m.runtime_mins.map(|r| r * 60);
                }
            }
        }
    }
}

/// A guide row or a recording, as the details merge needs it.
struct Airing {
    source_id: i64,
    kind: &'static str,
    title: String,
    subtitle: Option<String>,
    description: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    flags: i64,
    ext_id: Option<String>,
    runtime_mins: Option<i64>,
}

/// What to ask TMDB for an airing: the server's ids when it has them, else title + episode.
fn airing_lookup(a: &Airing, x: &ProgrammeExtra) -> Lookup {
    let has_episode = a.season.is_some() && a.episode.is_some();
    let hint = match x.tmdb_is_movie {
        Some(true) => Hint::Movie,
        Some(false) => Hint::Tv,
        None if has_episode => Hint::Tv,
        None if movie_categories(&x.categories) => Hint::Movie,
        None if skip_categories(&x.categories) || a.flags & flags::LIVE != 0 => Hint::Skip,
        None => Hint::Unknown,
    };
    Lookup {
        hint,
        title: a.title.clone(),
        year: x.production_date.as_deref().and_then(year_of),
        tmdb_id: x.tmdb_id.as_deref().and_then(|t| t.parse().ok()),
        imdb_id: x.imdb_id.clone(),
        season: a.season,
        episode: a.episode,
    }
}

fn push_crew(out: &mut Vec<CrewDto>, job: &'static str, names: Vec<String>) {
    let mut names: Vec<String> = names.into_iter().filter(|n| !n.trim().is_empty()).collect();
    names.dedup();
    if !names.is_empty() {
        out.push(CrewDto { job, names });
    }
}

fn tmdb_page(t: &TmdbTitle) -> String {
    let k = t.kind.unwrap_or(TmdbKind::Tv).as_str();
    format!("https://www.themoviedb.org/{k}/{}", t.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_titles() {
        assert!(generic_episode_title("Episode 3", "Show"));
        assert!(generic_episode_title("ep 12", "Show"));
        assert!(generic_episode_title("Show", "Show"));
        assert!(generic_episode_title("Show - S01E03", "Show"));
        assert!(generic_episode_title("s1e3", "Show"));
        assert!(generic_episode_title("", "Show"));
        assert!(!generic_episode_title("The Lantern Keeper", "Show"));
        assert!(!generic_episode_title("Episodes of Doubt", "Show"));
        assert!(!generic_episode_title("Seven Seas", "Show"));
    }

    #[test]
    fn category_hints() {
        assert!(skip_categories(&["News".into()]));
        assert!(skip_categories(&["Drama".into(), "Sports".into()]));
        assert!(!skip_categories(&["Drama".into()]));
        assert!(movie_categories(&["Movie".into()]));
        assert!(!movie_categories(&["Movies & Stuff".into()]));
    }

    #[test]
    fn trailer_urls_need_a_real_id() {
        assert_eq!(
            trailer_url(Some("trailerKey1")).as_deref(),
            Some("https://www.youtube.com/watch?v=trailerKey1")
        );
        assert_eq!(trailer_url(Some("x")), None);
        assert_eq!(trailer_url(Some("bad key/../")), None);
    }

    #[test]
    fn source_text_wins() {
        assert_eq!(
            pick_text(Some("a".into()), Some("b".into())).as_deref(),
            Some("a")
        );
        assert_eq!(
            pick_text(Some(" ".into()), Some("b".into())).as_deref(),
            Some("b")
        );
        assert_eq!(pick_text(None, None), None);
        assert_eq!(
            genres_of(Some("Drama / Crime, Mystery")),
            vec!["Drama", "Crime", "Mystery"]
        );
    }
}
