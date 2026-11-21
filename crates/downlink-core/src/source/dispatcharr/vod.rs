//! Dispatcharr VOD and DVR endpoints (PLAN §3.4, M4). Shapes follow the OpenAPI schema
//! (`Movie`, `Series`, `Episode`, `Recording`, `RecurringRecordingRule`, series rules); every field
//! is optional and unknown fields are ignored. `/proxy/vod/…` and the recording file/HLS URLs are
//! only ever *built* here; they are requested by the player at an actual play.

use reqwest::Method;
use serde_json::{Value, json};

use super::dto::{flex_i64, flex_string, parse_time};
use super::{Credentials, DispatcharrClient, fmt_time};
use crate::auth::SecretString;
use crate::error::{AppError, Result};
use crate::redact::RedactionSet;
use crate::source::vod::*;
use crate::source::{PlayPlan, PlayProfile, Surface};

/// Page size asked of the VOD list endpoints (the reference server's cap; a smaller cap is detected).
const VOD_PAGE_SIZE: usize = 100;
/// Upper bound on server pages fetched for one window (500 items at the smallest sane page size).
const MAX_VOD_PAGES: usize = 60;

/// The only fields we read from an episode. Anything else (notably `providers[]`) is skipped by
/// serde without being stored.
#[derive(serde::Deserialize, Default)]
struct EpisodeRow {
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    id: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    uuid: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    name: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    description: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    rating: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_i64")]
    duration_secs: Option<i64>,
    #[serde(default, deserialize_with = "super::dto::de_opt_i64")]
    season_number: Option<i64>,
    #[serde(default, deserialize_with = "super::dto::de_opt_i64")]
    episode_number: Option<i64>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    tmdb_id: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    imdb_id: Option<String>,
}

#[derive(serde::Deserialize)]
struct EpisodeEnvelope {
    #[serde(default)]
    results: Vec<EpisodeRow>,
    #[serde(default)]
    next: Option<String>,
}

impl EpisodeRow {
    fn into_item(self, series_ext_id: &str) -> Option<VodItem> {
        Some(VodItem {
            kind: Some(VodKind::Episode),
            ext_id: self.id?,
            uuid: self.uuid,
            parent_ext: Some(series_ext_id.to_owned()),
            title: self.name.unwrap_or_default(),
            rating: self.rating,
            plot: self.description,
            season: self.season_number,
            episode: self.episode_number,
            duration_secs: self.duration_secs.filter(|d| *d > 0),
            tmdb_id: clean_tmdb(self.tmdb_id),
            imdb_id: clean_imdb(self.imdb_id),
            ..Default::default()
        })
    }
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(flex_string)
}

fn first_http(v: Option<&Value>) -> Option<String> {
    let c = match v? {
        Value::Array(a) => a.iter().find_map(flex_string)?,
        other => flex_string(other)?,
    };
    (c.starts_with("http://") || c.starts_with("https://")).then_some(c)
}

/// Fields of `provider-info` we use. The reply also holds an `m3u_account` summary and more; serde
/// skips what is not named here.
#[derive(serde::Deserialize, Default)]
struct ProviderInfo {
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    description: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    plot: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_i64")]
    year: Option<i64>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    genre: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_string")]
    rating: Option<String>,
    #[serde(default, deserialize_with = "super::dto::de_opt_i64")]
    duration_secs: Option<i64>,
    #[serde(default)]
    backdrop_path: Option<Value>,
}

impl DispatcharrClient {
    /// Rewrite an absolute URL the server built from the request Host (`cache_url`, image URLs) onto
    /// our configured base when it is one of its own `/api/` paths; relative paths are joined;
    /// anything else (a poster CDN) stays as it is.
    fn server_url(&self, raw: &str) -> Option<String> {
        if raw.starts_with("http://") || raw.starts_with("https://") {
            let u = reqwest::Url::parse(raw).ok()?;
            if u.path().starts_with("/api/")
                || (self.base.path() != "/" && u.path().starts_with(self.base.path()))
            {
                let tail = format!(
                    "{}{}",
                    u.path().trim_start_matches('/'),
                    u.query().map(|q| format!("?{q}")).unwrap_or_default()
                );
                return self.base.join(&tail).ok().map(|x| x.to_string());
            }
            return Some(raw.to_owned());
        }
        self.url(raw.trim_start_matches('/'))
            .ok()
            .map(|u| u.to_string())
    }

    /// `GET …/{id}/provider-info/`: the server asks the provider (`get_vod_info` /
    /// `get_series_info`) and fills in details; for a series it also creates the episode list. A
    /// control endpoint, not a stream. Parsed into typed fields only.
    pub async fn provider_info(&self, kind: VodKind, ext_id: &str) -> Result<Option<VodItem>> {
        let seg = if kind == VodKind::Movie {
            "movies"
        } else {
            "series"
        };
        let r = self
            .call(
                Method::GET,
                &format!("api/vod/{seg}/{ext_id}/provider-info/"),
                &[],
                None,
            )
            .await?;
        if r.status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let r = r.ok("that title's details")?;
        let info: ProviderInfo = serde_json::from_slice(&r.body)?;
        drop(r);
        Ok(Some(VodItem {
            kind: Some(kind),
            ext_id: ext_id.to_owned(),
            plot: info.description.or(info.plot).filter(|d| !d.is_empty()),
            year: info.year,
            genre: info.genre.filter(|g| !g.is_empty()),
            rating: info.rating.filter(|g| !g.is_empty()),
            duration_secs: info.duration_secs.filter(|d| *d > 0),
            backdrop: first_http(info.backdrop_path.as_ref())
                .map(|u| self.server_url(&u).unwrap_or(u)),
            ..Default::default()
        }))
    }

    // ---- VOD --------------------------------------------------------------------------------

    pub async fn vod_category_list(&self, kind: VodKind) -> Result<Vec<VodCategoryInfo>> {
        let ty = if kind == VodKind::Movie {
            "movie"
        } else {
            "series"
        };
        let items = self
            .fetch_all(
                "api/vod/categories/",
                &[("category_type", ty.to_owned())],
                "its categories",
            )
            .await?;
        let mut out = Vec::new();
        for c in items {
            // Some servers ignore the filter: keep only the requested type when it is reported.
            if c.get("category_type")
                .and_then(Value::as_str)
                .is_some_and(|t| t != ty)
            {
                continue;
            }
            let (Some(id), Some(name)) = (s(&c, "id"), s(&c, "name")) else {
                continue;
            };
            out.push(VodCategoryInfo {
                ext_id: id,
                name,
                count: None,
            });
        }
        Ok(out)
    }

    pub(super) async fn cached_vod_categories(
        &self,
        kind: VodKind,
    ) -> Result<Vec<VodCategoryInfo>> {
        let mut cache = self.vod_categories.lock().await;
        if let Some((fetched, categories)) = cache.get(&kind)
            && fetched.elapsed() < super::VOD_CATEGORY_TTL
        {
            return Ok(categories.clone());
        }
        let categories = self.vod_category_list(kind).await?;
        cache.insert(kind, (std::time::Instant::now(), categories.clone()));
        Ok(categories)
    }

    async fn category_name(&self, kind: VodKind, ext_id: &str) -> Result<Option<String>> {
        Ok(self
            .cached_vod_categories(kind)
            .await?
            .into_iter()
            .find(|c| c.ext_id == ext_id)
            .map(|c| c.name))
    }

    /// Item count of a category (one request, page size 1).
    pub async fn vod_category_count(&self, kind: VodKind, name: &str) -> Result<i64> {
        let path = if kind == VodKind::Movie {
            "api/vod/movies/"
        } else {
            "api/vod/series/"
        };
        let (_, count, _) = self
            .page_values(path, &[("category", name.to_owned())], 1, 1, "its library")
            .await?;
        Ok(count.map_or(0, |c| i64::try_from(c).unwrap_or(0)))
    }

    fn vod_item_from(&self, kind: VodKind, v: &Value) -> Option<VodItem> {
        let id = s(v, "id")?;
        // Art: the logo's own cache URL (same server, authenticated) is preferred; otherwise its
        // external URL (fetched anonymously).
        let poster = v.get("logo").filter(|l| l.is_object()).and_then(|l| {
            l.get("cache_url")
                .and_then(flex_string)
                .filter(|c| !c.is_empty())
                .and_then(|c| self.server_url(&c))
                .or_else(|| first_http(l.get("url")))
        });
        // `custom_properties` is an object on most rows but a JSON *string* in `/api/vod/all/`.
        let cp_owned: Option<Value> = match v.get("custom_properties") {
            Some(Value::String(t)) => serde_json::from_str(t).ok(),
            Some(o) => Some(o.clone()),
            None => None,
        };
        let cp = cp_owned.as_ref();
        Some(VodItem {
            kind: Some(kind),
            uuid: s(v, "uuid"),
            parent_ext: v.get("series").and_then(|x| {
                if x.is_object() {
                    x.get("id").and_then(flex_string)
                } else {
                    flex_string(x)
                }
            }),
            title: s(v, "name").unwrap_or_default(),
            year: v.get("year").and_then(flex_i64).or_else(|| {
                cp.and_then(|c| c.get("releaseDate"))
                    .and_then(flex_string)
                    .and_then(|d| d.get(..4)?.parse().ok())
            }),
            rating: s(v, "rating"),
            genre: s(v, "genre"),
            plot: s(v, "description"),
            season: v.get("season_number").and_then(flex_i64),
            episode: v.get("episode_number").and_then(flex_i64),
            duration_secs: v.get("duration_secs").and_then(flex_i64).filter(|d| *d > 0),
            poster,
            backdrop: first_http(cp.and_then(|c| c.get("backdrop_path")))
                .map(|u| self.server_url(&u).unwrap_or(u)),
            category_ext: None,
            container: None,
            tmdb_id: clean_tmdb(s(v, "tmdb_id")),
            imdb_id: clean_imdb(s(v, "imdb_id")),
            ext_id: id,
        })
    }

    pub async fn vod_page(&self, kind: VodKind, q: &VodListQuery) -> Result<VodListPage> {
        let path = if kind == VodKind::Movie {
            "api/vod/movies/"
        } else {
            "api/vod/series/"
        };
        let mut extra: Vec<(&str, String)> = Vec::new();
        if let Some(cat) = &q.category_ext
            && let Some(name) = self.category_name(kind, cat).await?
        {
            extra.push(("category", name));
        }
        if let Some(search) = q.search.as_deref().filter(|x| !x.trim().is_empty()) {
            extra.push(("search", search.to_owned()));
        }
        // The server pages by number and caps `page_size` (100 on the reference server) without
        // saying so: walk its pages from the one holding `offset` until `limit` items are in hand.
        let limit = usize::try_from(q.limit.clamp(1, 500)).unwrap_or(100);
        let offset = usize::try_from(q.offset.max(0)).unwrap_or(0);
        let mut size = VOD_PAGE_SIZE;
        let mut raw: Vec<Value> = Vec::new();
        let mut total = None;
        let mut page = offset / size + 1;
        let mut skip = offset % size;
        for _ in 0..MAX_VOD_PAGES {
            let (items, count, has_next) = self
                .page_values(
                    path,
                    &extra,
                    u32::try_from(page).unwrap_or(1),
                    u32::try_from(size).unwrap_or(100),
                    "its library",
                )
                .await?;
            total = count.or(total);
            if has_next && !items.is_empty() && items.len() < size {
                // A smaller cap than asked for: restart from the right page at that size.
                size = items.len();
                let pos = offset + raw.len();
                page = pos / size + 1;
                skip = pos % size;
                continue;
            }
            let got = items.len();
            raw.extend(items.into_iter().skip(skip));
            skip = 0;
            if raw.len() >= limit || !has_next || got == 0 {
                break;
            }
            page += 1;
        }
        raw.truncate(limit);
        let items: Vec<VodItem> = raw
            .iter()
            .filter_map(|v| self.vod_item_from(kind, v))
            .collect();
        Ok(VodListPage {
            items,
            total: total.map(|c| i64::try_from(c).unwrap_or(0)),
        })
    }

    pub async fn vod_item_detail(&self, kind: VodKind, ext_id: &str) -> Result<VodItem> {
        let path = if kind == VodKind::Movie {
            "movies"
        } else {
            "series"
        };
        let v = self
            .get_json(&format!("api/vod/{path}/{ext_id}/"), &[], "that title")
            .await?;
        self.vod_item_from(kind, &v)
            .ok_or_else(|| AppError::server("The server sent a title Downlink could not read."))
    }

    /// Episodes of a series. The reference server answers with a bare array in which every
    /// episode carries `providers[]` holding the *whole* M3U account object (provider username and
    /// password, server URL, hundreds of channel groups), repeated per episode: megabytes. So this
    /// parses the body straight into a typed struct that names only the fields we use (serde skips
    /// the rest without materialising it), never builds a `Value` of the body, never logs it and
    /// never puts any of it in an error.
    pub async fn series_episode_list(&self, series_ext_id: &str) -> Result<Vec<VodItem>> {
        let path = format!("api/vod/series/{series_ext_id}/episodes/");
        let mut eps: Vec<VodItem> = Vec::new();
        for page in 1..=200u32 {
            let q = [("page", page.to_string()), ("page_size", "500".to_owned())];
            let reply = self
                .call(Method::GET, &path, &q, None)
                .await?
                .ok("that series")?;
            let first = reply
                .body
                .iter()
                .copied()
                .find(|b| !b.is_ascii_whitespace());
            let (rows, has_next) = match first {
                Some(b'[') => (
                    serde_json::from_slice::<Vec<EpisodeRow>>(&reply.body)?,
                    false,
                ),
                Some(b'{') => {
                    let env = serde_json::from_slice::<EpisodeEnvelope>(&reply.body)?;
                    (env.results, env.next.is_some_and(|n| !n.is_empty()))
                }
                _ => (Vec::new(), false),
            };
            drop(reply);
            let empty = rows.is_empty();
            eps.extend(rows.into_iter().filter_map(|r| r.into_item(series_ext_id)));
            if !has_next || empty {
                break;
            }
        }
        eps.sort_by_key(|e| (e.season.unwrap_or(0), e.episode.unwrap_or(0)));
        Ok(eps)
    }

    /// `/proxy/vod/{movie|episode}/{uuid}` plan: built, never requested here.
    ///
    /// The VOD proxy does not need credentials. A JWT is not sent: mpv repeats the header on every
    /// seek and reconnect of a two-hour film, long after the 30-minute access token expired, and the
    /// server rejects an expired token outright (even where none is required). An API key does not
    /// expire and is still sent.
    pub async fn vod_plan(&self, item: &VodItem) -> Result<PlayPlan> {
        let kind = match item.kind {
            Some(VodKind::Movie) => "movie",
            Some(VodKind::Episode) => "episode",
            _ => return Err(AppError::stream("Only movies and episodes can be played.")),
        };
        let uuid = item.uuid.as_deref().ok_or_else(|| {
            AppError::stream("This title has no stream address yet; refresh the library.")
        })?;
        let url = self.base.join(&format!("proxy/vod/{kind}/{uuid}"))?;
        let mut redact: RedactionSet = self.redaction_set();
        let mut headers = Vec::new();
        if let Credentials::ApiKey(k) = &self.creds {
            redact.add(k.expose());
            headers.push(("X-API-Key".to_owned(), k.clone()));
        }
        Ok(PlayPlan {
            url: url.to_string(),
            headers,
            surface: Surface::Direct,
            profile: PlayProfile::Seekable,
            session: None,
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        })
    }

    // ---- DVR --------------------------------------------------------------------------------

    fn recording_from(&self, v: &Value, now: i64) -> Option<RecordingInfo> {
        let ext_id = s(v, "id")?;
        // The recording's own times include pre/post padding; those drive its state. The
        // programme's own times are what the user recognises, so they are what we display.
        let rec_start = v.get("start_time").and_then(parse_time)?;
        let rec_end = v.get("end_time").and_then(parse_time)?;
        let cp = v.get("custom_properties").cloned().unwrap_or(Value::Null);
        let program = cp.get("program").cloned().unwrap_or(Value::Null);
        let (start, end) = match (
            program.get("start_time").and_then(parse_time),
            program.get("end_time").and_then(parse_time),
        ) {
            (Some(s), Some(e)) if e > s => (s, e),
            _ => (rec_start, rec_end),
        };
        let title = s(&program, "title")
            .or_else(|| s(&cp, "title"))
            .unwrap_or_else(|| "Recording".to_owned());
        let status_s = s(&cp, "status").map(|x| x.to_ascii_lowercase());
        // `status` is absent while a one-off or series recording is scheduled: derive from times.
        let by_time = |now: i64| {
            if now < rec_start {
                RecStatus::Scheduled
            } else if now < rec_end {
                RecStatus::Recording
            } else {
                RecStatus::Completed
            }
        };
        let mut interrupted = false;
        let status = match status_s.as_deref() {
            Some("scheduled" | "pending" | "upcoming") => RecStatus::Scheduled,
            Some("recording" | "in_progress" | "running" | "active") => RecStatus::Recording,
            Some("failed" | "error") => RecStatus::Failed,
            // Stopped early: the partial file is playable, so it is a finished recording.
            Some("completed" | "complete" | "done" | "finished" | "stopped") => {
                RecStatus::Completed
            }
            Some("interrupted") => {
                interrupted = true;
                RecStatus::Failed
            }
            // Cancelled recordings are gone as far as the user is concerned.
            Some("cancelled" | "canceled") => return None,
            _ => by_time(now),
        };
        let rule = cp.get("rule").filter(|r| r.is_object());
        let rule_ext = rule.and_then(|r| s(r, "id"));
        let rule_label = if rule.and_then(|r| r.get("type")).and_then(Value::as_str)
            == Some("recurring")
            || cp.get("recurring_rule_id").is_some()
        {
            Some("Recurring".to_owned())
        } else if cp.get("rule_id").is_some() || cp.get("series_rule").is_some() {
            Some("Series".to_owned())
        } else {
            None
        };
        Some(RecordingInfo {
            ext_id,
            channel_ext_id: s(v, "channel")?,
            title,
            start_utc: start,
            end_utc: end,
            status,
            rule_label,
            size_bytes: ["bytes_written", "file_size", "size_bytes", "size"]
                .iter()
                .find_map(|k| cp.get(*k).and_then(flex_i64)),
            error: (status == RecStatus::Failed).then(|| {
                if interrupted {
                    "The recording was interrupted.".to_owned()
                } else {
                    s(&cp, "error")
                        .or_else(|| s(&cp, "message"))
                        .unwrap_or_else(|| "The recording failed.".into())
                }
            }),
            rule_ext,
            tvg_id: s(&program, "tvg_id"),
            file_url: s(&cp, "file_url"),
        })
    }

    pub async fn recording_list(&self) -> Result<Vec<RecordingInfo>> {
        let items = self
            .fetch_all("api/channels/recordings/", &[], "its recordings")
            .await?;
        let now = crate::sync::now_utc();
        Ok(items
            .iter()
            .filter_map(|v| self.recording_from(v, now))
            .collect())
    }

    pub async fn create_recording(&self, r: &RecordRequest) -> Result<()> {
        let channel: i64 = r
            .channel_ext_id
            .parse()
            .map_err(|_| AppError::local("That channel cannot be recorded."))?;
        let body = json!({
            "channel": channel,
            "start_time": fmt_time(r.start_utc)?,
            "end_time": fmt_time(r.end_utc)?,
            "custom_properties": { "program": {
                "id": r.programme_ext_id, "tvg_id": r.tvg_id, "title": r.title, "description": r.description,
                "start_time": fmt_time(r.start_utc)?, "end_time": fmt_time(r.end_utc)?,
            }},
        });
        let r = self
            .call(Method::POST, "api/channels/recordings/", &[], Some(&body))
            .await?;
        if r.status == reqwest::StatusCode::BAD_REQUEST {
            // The body is only inspected, never echoed.
            let text = String::from_utf8_lossy(&r.body);
            return Err(if text.contains("must be in the future") {
                AppError::stream("That programme has already finished, so it cannot be recorded.")
            } else {
                AppError::stream("The server did not accept that recording.")
            });
        }
        r.ok("recording")?;
        Ok(())
    }

    /// The EPG source of the guide data a channel is assigned to (`channel.epg_data_id` ->
    /// `epgdata.epg_source`). Best effort: `None` when the channel has no guide data or the server
    /// does not say.
    async fn channel_epg_source(&self, channel_ext_id: &str) -> Option<i64> {
        let ch = self
            .get_json(
                &format!("api/channels/channels/{channel_ext_id}/"),
                &[],
                "that channel",
            )
            .await
            .ok()?;
        let data_id = ["effective_epg_data_id", "epg_data_id"]
            .iter()
            .find_map(|k| ch.get(k).and_then(flex_i64))?;
        let d = self
            .get_json(
                &format!("api/epg/epgdata/{data_id}/"),
                &[],
                "that channel's guide data",
            )
            .await
            .ok()?;
        ["epg_source", "epg_source_id"]
            .iter()
            .find_map(|k| d.get(k).and_then(flex_i64))
    }

    pub async fn create_series_rule(&self, r: &RecordRequest) -> Result<()> {
        let tvg = r
            .tvg_id
            .as_deref()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                AppError::local("This channel has no guide id, so a series rule cannot be created.")
            })?;
        let mut body = json!({
            "tvg_id": tvg, "mode": "new", "title": r.title, "title_mode": "exact",
            "channel_id": r.channel_ext_id.parse::<i64>().ok(),
        });
        // Scope the rule to the guide source the channel is assigned to, when it can be told.
        if let Some(src) = self.channel_epg_source(&r.channel_ext_id).await {
            body["epg_source_id"] = json!(src);
        }
        if let Some(d) = r.description.as_deref().filter(|d| !d.is_empty()) {
            body["description"] = json!(d);
            body["description_mode"] = json!("contains");
        }
        self.call(Method::POST, "api/channels/series-rules/", &[], Some(&body))
            .await?
            .ok("the series rule")?;
        // Creating a rule schedules nothing by itself: evaluate so matching programmes get
        // recordings. Best effort; the rule exists either way.
        if let Err(e) = self
            .call(
                Method::POST,
                "api/channels/series-rules/evaluate/",
                &[],
                Some(&json!({})),
            )
            .await
            .and_then(|r| r.ok("the series rule"))
        {
            tracing::warn!(error = %e, "series rule evaluation failed");
        }
        Ok(())
    }

    pub async fn stop_recording_by_id(&self, id: &str) -> Result<()> {
        self.call(
            Method::POST,
            &format!("api/channels/recordings/{id}/stop/"),
            &[],
            Some(&json!({})),
        )
        .await?
        .ok("that recording")?;
        Ok(())
    }

    /// Idempotent (404 = already gone).
    pub async fn delete_recording_by_id(&self, id: &str) -> Result<()> {
        let r = self
            .call(
                Method::DELETE,
                &format!("api/channels/recordings/{id}/"),
                &[],
                None,
            )
            .await?;
        if r.status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        r.ok("that recording")?;
        Ok(())
    }

    pub async fn extend_recording_by_id(&self, id: &str, minutes: i64) -> Result<()> {
        if !(1..=24 * 60).contains(&minutes) {
            return Err(AppError::local(
                "Extend a recording by between 1 minute and 24 hours.",
            ));
        }
        let r = self
            .call(
                Method::POST,
                &format!("api/channels/recordings/{id}/extend/"),
                &[],
                Some(&json!({ "extra_minutes": minutes })),
            )
            .await?;
        if r.status == reqwest::StatusCode::BAD_REQUEST {
            let text = String::from_utf8_lossy(&r.body);
            return Err(AppError::stream(if text.contains("already finished") {
                "That recording has already finished."
            } else {
                "The server would not extend that recording."
            }));
        }
        r.ok("that recording")?;
        Ok(())
    }

    /// Plan for a recording: HLS while it runs, the file when complete. Authenticated with the
    /// source key (same-origin media, not a provider stream).
    pub async fn recording_play_plan(&self, rec: &RecordingInfo) -> Result<PlayPlan> {
        // While recording, the EVENT playlist is the only seekable-growing source (`/file/` would
        // just 302 to it); afterwards the MKV.
        let rel = if rec.status == RecStatus::Recording {
            format!("api/channels/recordings/{}/hls/index.m3u8", rec.ext_id)
        } else {
            match rec.file_url.as_deref().filter(|u| !u.is_empty()) {
                Some(u) => u.to_owned(),
                None => format!("api/channels/recordings/{}/file/", rec.ext_id),
            }
        };
        let url = if rel.starts_with("http://") || rel.starts_with("https://") {
            let u = reqwest::Url::parse(&rel)?;
            if u.origin() != self.base.origin() {
                return Err(AppError::stream("This recording points at another server."));
            }
            u.to_string()
        } else {
            self.base.join(rel.trim_start_matches('/'))?.to_string()
        };
        let auth: (String, SecretString) = self.auth_header().await?;
        let mut redact = self.redaction_set();
        redact.add(auth.1.expose());
        let hls = url.contains(".m3u8");
        Ok(PlayPlan {
            url,
            headers: vec![auth],
            surface: if hls { Surface::Hls } else { Surface::Direct },
            profile: PlayProfile::Seekable,
            session: None,
            redact,
            needs_redirect_resolve: false,
            unknown_profile: false,
        })
    }

    pub async fn rule_list(&self) -> Result<Vec<RuleInfo>> {
        let mut out = Vec::new();
        // Recurring rules: bare array.
        let rec = self
            .fetch_all("api/channels/recurring-rules/", &[], "its recurring rules")
            .await
            .unwrap_or_default();
        for r in &rec {
            let Some(id) = s(r, "id") else { continue };
            let days: Vec<i64> = r
                .get("days_of_week")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(flex_i64).collect())
                .unwrap_or_default();
            let at = s(r, "start_time")
                .map(|t| t.chars().take(5).collect::<String>())
                .unwrap_or_default();
            out.push(RuleInfo {
                kind: RuleKind::Recurring,
                ext_id: id,
                title: s(r, "name").unwrap_or_else(|| "Recurring recording".into()),
                channel_name: String::new(),
                channel_ext_id: s(r, "channel"),
                tvg_id: None,
                schedule: format!("{} {at}", day_label(&days)).trim().to_owned(),
            });
        }
        // Series rules: `{ "rules": [...] }`.
        let v = self
            .get_json("api/channels/series-rules/", &[], "its series rules")
            .await
            .unwrap_or(Value::Null);
        let rules = v
            .get("rules")
            .and_then(Value::as_array)
            .cloned()
            .or_else(|| v.as_array().cloned())
            .unwrap_or_default();
        for r in &rules {
            let tvg = s(r, "tvg_id").unwrap_or_default();
            let title = s(r, "title").unwrap_or_default();
            if tvg.is_empty() && title.is_empty() {
                continue;
            }
            out.push(RuleInfo {
                kind: RuleKind::Series,
                ext_id: format!("{tvg}|{title}"),
                title: if title.is_empty() { tvg.clone() } else { title },
                channel_name: String::new(),
                channel_ext_id: s(r, "channel_id"),
                tvg_id: (!tvg.is_empty()).then_some(tvg),
                schedule: if s(r, "mode").as_deref() == Some("all") {
                    "All episodes".into()
                } else {
                    "New episodes".into()
                },
            });
        }
        Ok(out)
    }

    pub async fn delete_rule_by(&self, rule: &RuleInfo) -> Result<()> {
        let r = match rule.kind {
            RuleKind::Recurring => {
                self.call(
                    Method::DELETE,
                    &format!("api/channels/recurring-rules/{}/", rule.ext_id),
                    &[],
                    None,
                )
                .await?
            }
            RuleKind::Series => {
                let (tvg, title) = rule
                    .ext_id
                    .split_once('|')
                    .unwrap_or((rule.ext_id.as_str(), ""));
                self.call(
                    Method::DELETE,
                    "api/channels/series-rules/",
                    &[("tvg_id", tvg.to_owned()), ("title", title.to_owned())],
                    None,
                )
                .await?
            }
        };
        if r.status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        r.ok("that rule")?;
        Ok(())
    }

    /// VOD/DVR capabilities for this account, probed once at connect.
    pub(super) async fn probe_features(&self, level: Option<crate::source::UserLevel>) {
        let dvr = !matches!(level, Some(crate::source::UserLevel::Streamer));
        let mut vod = false;
        for path in ["api/vod/movies/", "api/vod/series/"] {
            if let Ok((_, count, _)) = self.page_values(path, &[], 1, 1, "its library").await
                && count.unwrap_or(0) > 0
            {
                vod = true;
                break;
            }
        }
        self.has_vod
            .store(vod, std::sync::atomic::Ordering::Relaxed);
        self.has_dvr
            .store(dvr, std::sync::atomic::Ordering::Relaxed);
        self.can_switch.store(
            matches!(level, Some(crate::source::UserLevel::Admin)),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

fn day_label(days: &[i64]) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut d: Vec<i64> = days.to_vec();
    d.sort_unstable();
    d.dedup();
    match d.as_slice() {
        [] => "Daily".into(),
        [0, 1, 2, 3, 4, 5, 6] => "Daily".into(),
        [0, 1, 2, 3, 4] => "Weekdays".into(),
        [5, 6] => "Weekends".into(),
        other => other
            .iter()
            .filter_map(|i| usize::try_from(*i).ok().and_then(|i| NAMES.get(i)))
            .copied()
            .collect::<Vec<_>>()
            .join(", "),
    }
}
