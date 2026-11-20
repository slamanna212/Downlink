//! Dispatcharr VOD and DVR endpoints (PLAN §3.4, M4). Shapes follow the OpenAPI schema
//! (`Movie`, `Series`, `Episode`, `Recording`, `RecurringRecordingRule`, series rules); every field
//! is optional and unknown fields are ignored. `/proxy/vod/…` and the recording file/HLS URLs are
//! only ever *built* here; they are requested by the player at an actual play.

use reqwest::Method;
use serde_json::{Value, json};

use super::dto::{flex_i64, flex_string, parse_time};
use super::{DispatcharrClient, fmt_time};
use crate::auth::SecretString;
use crate::error::{AppError, Result};
use crate::redact::RedactionSet;
use crate::source::vod::*;
use crate::source::{PlayPlan, PlayProfile, Surface};

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

impl DispatcharrClient {
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

    async fn category_name(&self, kind: VodKind, ext_id: &str) -> Result<Option<String>> {
        Ok(self
            .vod_category_list(kind)
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
        let img_kind = match kind {
            VodKind::Movie => "movies",
            VodKind::Series => "series",
            VodKind::Episode => "episodes",
        };
        let has_logo = v.get("logo").is_some_and(|l| !l.is_null());
        let poster = has_logo
            .then(|| {
                self.url(&format!("api/vod/{img_kind}/{id}/image"))
                    .ok()
                    .map(|u| u.to_string())
            })
            .flatten();
        let cp = v.get("custom_properties");
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
            year: v.get("year").and_then(flex_i64),
            rating: s(v, "rating"),
            genre: s(v, "genre"),
            plot: s(v, "description"),
            season: v.get("season_number").and_then(flex_i64),
            episode: v.get("episode_number").and_then(flex_i64),
            duration_secs: v.get("duration_secs").and_then(flex_i64).filter(|d| *d > 0),
            poster,
            backdrop: first_http(cp.and_then(|c| c.get("backdrop_path"))),
            category_ext: None,
            container: None,
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
        // The server pages by number: fetch whole pages and slice to the requested window.
        let limit = q.limit.clamp(1, 500);
        let offset = q.offset.max(0);
        let (page, size, skip) = if offset % limit == 0 {
            (offset / limit + 1, limit, 0)
        } else {
            let size = (offset + limit).min(500);
            (1, size, offset.min(size))
        };
        let (items, count, _) = self
            .page_values(
                path,
                &extra,
                u32::try_from(page).unwrap_or(1),
                u32::try_from(size).unwrap_or(50),
                "its library",
            )
            .await?;
        let items: Vec<VodItem> = items
            .iter()
            .filter_map(|v| self.vod_item_from(kind, v))
            .skip(usize::try_from(skip).unwrap_or(0))
            .take(usize::try_from(limit).unwrap_or(50))
            .collect();
        Ok(VodListPage {
            items,
            total: count.map(|c| i64::try_from(c).unwrap_or(0)),
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

    pub async fn series_episode_list(&self, series_ext_id: &str) -> Result<Vec<VodItem>> {
        let items = self
            .fetch_all(
                &format!("api/vod/series/{series_ext_id}/episodes/"),
                &[],
                "that series",
            )
            .await?;
        let mut eps: Vec<VodItem> = items
            .iter()
            .filter_map(|v| self.vod_item_from(VodKind::Episode, v))
            .collect();
        for e in &mut eps {
            e.parent_ext = Some(series_ext_id.to_owned());
        }
        eps.sort_by_key(|e| (e.season.unwrap_or(0), e.episode.unwrap_or(0)));
        Ok(eps)
    }

    /// `/proxy/vod/{movie|episode}/{uuid}` plan: built, never requested here.
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
        let auth = self.auth_header().await?;
        let mut redact: RedactionSet = self.redaction_set();
        redact.add(auth.1.expose());
        Ok(PlayPlan {
            url: url.to_string(),
            headers: vec![auth],
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
        let status = match status_s.as_deref() {
            Some("scheduled" | "pending" | "upcoming") => RecStatus::Scheduled,
            Some("recording" | "in_progress" | "running" | "active") => RecStatus::Recording,
            Some("failed" | "error") => RecStatus::Failed,
            Some("completed" | "complete" | "done" | "finished" | "stopped" | "interrupted") => {
                RecStatus::Completed
            }
            _ => {
                if now < rec_start {
                    RecStatus::Scheduled
                } else if now < rec_end {
                    RecStatus::Recording
                } else {
                    RecStatus::Completed
                }
            }
        };
        let rule_label = if cp.get("rule_id").is_some()
            || cp.get("series_rule").is_some()
            || cp.get("rule").and_then(Value::as_str) == Some("series")
        {
            Some("Series".to_owned())
        } else if cp.get("recurring_rule_id").is_some() {
            Some("Recurring".to_owned())
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
                s(&cp, "error")
                    .or_else(|| s(&cp, "message"))
                    .unwrap_or_else(|| "The recording failed.".into())
            }),
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
        self.call(Method::POST, "api/channels/recordings/", &[], Some(&body))
            .await?
            .ok("recording")?;
        Ok(())
    }

    pub async fn create_series_rule(&self, r: &RecordRequest) -> Result<()> {
        let tvg = r
            .tvg_id
            .as_deref()
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                AppError::local("This channel has no guide id, so a series rule cannot be created.")
            })?;
        let body = json!({
            "tvg_id": tvg, "mode": "new", "title": r.title, "title_mode": "exact",
            "channel_id": r.channel_ext_id.parse::<i64>().ok(),
        });
        self.call(Method::POST, "api/channels/series-rules/", &[], Some(&body))
            .await?
            .ok("the series rule")?;
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
        self.call(
            Method::POST,
            &format!("api/channels/recordings/{id}/extend/"),
            &[],
            Some(&json!({ "minutes": minutes })),
        )
        .await?
        .ok("that recording")?;
        Ok(())
    }

    /// Plan for a recording: HLS while it runs, the file when complete. Authenticated with the
    /// source key (same-origin media, not a provider stream).
    pub async fn recording_play_plan(&self, rec: &RecordingInfo) -> Result<PlayPlan> {
        let rel = match rec.file_url.as_deref().filter(|u| !u.is_empty()) {
            Some(u) => u.to_owned(),
            None if rec.status == RecStatus::Recording => {
                format!("api/channels/recordings/{}/hls/index.m3u8", rec.ext_id)
            }
            None => format!("api/channels/recordings/{}/file/", rec.ext_id),
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
