//! `sports_match` persistence (docs/SPORTS.md §4): the follow document in `app_kv`, the rebuild
//! for now .. +7 days, and the listing the commands read. Visibility (adult/radio/DRM) comes from
//! `catalog/`, never from here.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use sqlx::FromRow;

use super::catalog::{Catalog, Kind};
use super::matcher::{self, Candidate, Match};
use crate::catalog::{self, ChannelFilter};
use crate::error::{AppError, Result};
use crate::service::dto::SportsFollowDto;
use crate::source::flags;
use crate::store::{ChannelRow, ProgrammeRow, Store};

pub const FOLLOW_KEY: &str = "sports.follow";
/// How far ahead matches are kept.
pub const WINDOW_SECS: i64 = 7 * 86_400;
/// FTS phrases per MATCH query (one OR query per chunk).
const FTS_CHUNK: usize = 150;
/// Upper bound on visible channels scanned for event-channel names.
const MAX_CHANNELS: i64 = 200_000;

fn default_follow() -> SportsFollowDto {
    SportsFollowDto {
        version: 1,
        include_replays: false,
        leagues: Default::default(),
        series: Default::default(),
        priority: Vec::new(),
    }
}

async fn follow_raw(store: &Store) -> Result<Option<Value>> {
    Ok(store
        .kv_get(FOLLOW_KEY)
        .await?
        .and_then(|(v, _)| serde_json::from_str::<Value>(&v).ok())
        .filter(Value::is_object))
}

/// The saved follow document, or the default (version 1, no replays, nothing followed).
pub async fn load_follow(store: &Store) -> Result<SportsFollowDto> {
    Ok(follow_raw(store)
        .await?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_else(default_follow))
}

/// Save the follow document. Unknown top-level keys of the stored document are kept, and so are
/// league/series entries the catalog no longer knows (a catalog update never wipes picks).
pub async fn save_follow(
    store: &Store,
    cat: &Catalog,
    follow: &SportsFollowDto,
    now: i64,
) -> Result<()> {
    let mut out = follow_raw(store)
        .await?
        .unwrap_or_else(|| Value::Object(Default::default()));
    let old: SportsFollowDto =
        serde_json::from_value(out.clone()).unwrap_or_else(|_| default_follow());
    let mut merged = follow.clone();
    for (k, v) in old.leagues {
        if cat.league(&k).is_none() {
            merged.leagues.entry(k).or_insert(v);
        }
    }
    for (k, v) in old.series {
        if cat.league(&k).is_none() {
            merged.series.entry(k).or_insert(v);
        }
    }
    let Value::Object(fresh) = serde_json::to_value(&merged)? else {
        return Err(AppError::local("Sports settings could not be saved."));
    };
    if let Value::Object(m) = &mut out {
        m.extend(fresh);
    }
    store
        .kv_set(FOLLOW_KEY, &serde_json::to_string(&out)?, now)
        .await
}

/// FTS5 query: every phrase double-quoted (quotes inside removed), joined with OR. The raw text
/// goes in, so FTS tokenizes it the same way it tokenized the guide.
fn fts_phrase(raw: &str) -> Option<String> {
    let clean: String = raw.chars().filter(|c| *c != '"').collect();
    clean
        .chars()
        .any(char::is_alphanumeric)
        .then(|| format!("\"{clean}\""))
}

/// Tokens, context tokens, events and strong team aliases of the followed leagues.
fn candidate_phrases(cat: &Catalog, follow: &SportsFollowDto) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for l in &cat.leagues {
        let on = match l.kind {
            Kind::Team => follow.leagues.contains_key(&l.key),
            Kind::Motorsport => follow.series.contains_key(&l.key),
        };
        if !on {
            continue;
        }
        let phrases = l
            .tokens
            .iter()
            .chain(&l.context)
            .chain(&l.events)
            .chain(l.teams.iter().flat_map(|t| t.strong.iter()));
        for p in phrases {
            if let Some(q) = fts_phrase(&p.raw)
                && seen.insert(q.to_lowercase())
            {
                out.push(q);
            }
        }
    }
    out
}

/// A row to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchRow {
    pub channel_id: i64,
    pub start_utc: Option<i64>,
    pub end_utc: Option<i64>,
    pub m: Match,
    pub source: &'static str,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RebuildReport {
    pub candidates: usize,
    pub matches: usize,
}

/// Rebuild `sports_match` for `now .. now + 7 days`. Candidates: programmes found by one FTS OR
/// query of the followed leagues' phrases, plus every visible channel name. Only channels the
/// filter shows are considered. Scoring runs on a blocking thread.
pub async fn rebuild(
    store: &Store,
    cat: &'static Catalog,
    filter: &ChannelFilter,
    now: i64,
) -> Result<RebuildReport> {
    let follow = load_follow(store).await?;
    let phrases = candidate_phrases(cat, &follow);
    if phrases.is_empty() {
        sqlx::query("DELETE FROM sports_match")
            .execute(store.pool())
            .await?;
        return Ok(RebuildReport::default());
    }
    let end = now.saturating_add(WINDOW_SECS);

    let mut seen_rowids = HashSet::new();
    let mut programmes: Vec<ProgrammeRow> = Vec::new();
    for chunk in phrases.chunks(FTS_CHUNK) {
        let q = chunk.join(" OR ");
        let rows: Vec<(i64, ProgrammeRowSql)> = sqlx::query_as::<_, ProgrammeRowSql>(
            "SELECT p.rowid AS rowid, p.channel_id, p.start_utc, p.end_utc, p.ext_id, p.title,
                    p.subtitle, p.description, p.season, p.episode, p.flags, p.icon
             FROM programme_fts CROSS JOIN programme p ON p.rowid = programme_fts.rowid
             WHERE programme_fts MATCH ?1 AND p.end_utc > ?2 AND p.start_utc < ?3",
        )
        .bind(&q)
        .bind(now)
        .bind(end)
        .fetch_all(store.pool())
        .await?
        .into_iter()
        .map(|r| (r.rowid, r))
        .collect();
        for (rowid, r) in rows {
            if seen_rowids.insert(rowid) {
                programmes.push(r.into());
            }
        }
    }

    let channels = catalog::list_channels(store, filter, MAX_CHANNELS, 0).await?;
    let groups: HashMap<i64, String> =
        sqlx::query_as::<_, (i64, String)>("SELECT id, name FROM channel_group")
            .fetch_all(store.pool())
            .await?
            .into_iter()
            .collect();
    let sources = store.list_sources().await?;
    // Per source: does the guide mark live airings (LIVE flags here, title markers in
    // `score_all`)? If neither, live is inferred.
    let mut trusted: HashSet<i64> = HashSet::new();
    for s in &sources {
        let any_live: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM programme p JOIN channel c ON c.id = p.channel_id
             WHERE c.source_id = ?1 AND p.start_utc >= ?2 AND p.start_utc < ?3
               AND (p.flags & ?4) != 0)",
        )
        .bind(s.id)
        .bind(now - 86_400)
        .bind(end)
        .bind(flags::LIVE)
        .fetch_one(store.pool())
        .await?;
        if any_live {
            trusted.insert(s.id);
        }
    }
    let tz: HashMap<i64, chrono_tz::Tz> = sources
        .iter()
        .map(|s| (s.id, crate::epg::refresh::parse_tz(s.server_tz.as_deref())))
        .collect();

    let candidates = programmes.len() + channels.len();
    let rows = tokio::task::spawn_blocking(move || {
        score_all(
            cat,
            &follow,
            &programmes,
            &channels,
            &groups,
            &trusted,
            &tz,
            now,
        )
    })
    .await
    .map_err(|_| AppError::local("Sports matching stopped unexpectedly."))?;

    let mut tx = store.pool().begin().await?;
    sqlx::query("DELETE FROM sports_match")
        .execute(&mut *tx)
        .await?;
    for r in &rows {
        sqlx::query(
            "INSERT OR REPLACE INTO sports_match
               (channel_id, start_utc, end_utc, league, teams, session, live, live_inferred, score, source)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(r.channel_id)
        .bind(r.start_utc)
        .bind(r.end_utc)
        .bind(&r.m.league)
        .bind(r.m.teams.join(","))
        .bind(r.m.session)
        .bind(r.m.live)
        .bind(r.m.live_inferred)
        .bind(r.m.score)
        .bind(r.source)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(RebuildReport {
        candidates,
        matches: rows.len(),
    })
}

#[derive(FromRow)]
struct ProgrammeRowSql {
    rowid: i64,
    channel_id: i64,
    start_utc: i64,
    end_utc: i64,
    ext_id: Option<String>,
    title: Option<String>,
    subtitle: Option<String>,
    description: Option<String>,
    season: Option<i64>,
    episode: Option<i64>,
    flags: i64,
    icon: Option<String>,
}

impl From<ProgrammeRowSql> for ProgrammeRow {
    fn from(r: ProgrammeRowSql) -> Self {
        Self {
            channel_id: r.channel_id,
            start_utc: r.start_utc,
            end_utc: r.end_utc,
            ext_id: r.ext_id,
            title: r.title,
            subtitle: r.subtitle,
            description: r.description,
            season: r.season,
            episode: r.episode,
            flags: r.flags,
            icon: r.icon,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn score_all(
    cat: &Catalog,
    follow: &SportsFollowDto,
    programmes: &[ProgrammeRow],
    channels: &[ChannelRow],
    groups: &HashMap<i64, String>,
    trusted: &HashSet<i64>,
    tz: &HashMap<i64, chrono_tz::Tz>,
    now: i64,
) -> Vec<MatchRow> {
    let by_id: HashMap<i64, &ChannelRow> = channels.iter().map(|c| (c.id, c)).collect();
    let group_of = |c: &ChannelRow| c.group_id.and_then(|g| groups.get(&g)).map(String::as_str);
    // A source whose sports listings carry live markers ("ᴸᶦᵛᵉ", "(Live)") marks live airings:
    // trust it, so an unmarked airing is a replay.
    let mut trusted = trusted.clone();
    for p in programmes {
        if let Some(ch) = by_id.get(&p.channel_id)
            && matcher::live_marker(p.title.as_deref().unwrap_or(""), p.subtitle.as_deref())
        {
            trusted.insert(ch.source_id);
        }
    }
    let mut out = Vec::new();
    // (channel, league) pairs that already have a programme match: no extra channel-name row.
    let mut covered: HashSet<(i64, String)> = HashSet::new();
    for p in programmes {
        let Some(ch) = by_id.get(&p.channel_id) else {
            continue; // hidden by the catalog filter
        };
        let cand = Candidate {
            title: p.title.as_deref().unwrap_or(""),
            subtitle: p.subtitle.as_deref(),
            description: p.description.as_deref(),
            channel_name: &ch.name,
            group_name: group_of(ch),
            categories: &[],
            flags: p.flags,
        };
        for m in matcher::match_programme(cat, follow, &cand, trusted.contains(&ch.source_id)) {
            covered.insert((ch.id, m.league.clone()));
            out.push(MatchRow {
                channel_id: ch.id,
                start_utc: Some(p.start_utc),
                end_utc: Some(p.end_utc),
                m,
                source: "programme",
            });
        }
    }
    demote_repeats(programmes, &mut out);
    demote_team_reairs(cat, &mut out);
    for ch in channels {
        let ms = matcher::match_channel(cat, follow, &ch.name, group_of(ch));
        if ms.is_empty() {
            continue;
        }
        let zone = tz.get(&ch.source_id).copied().unwrap_or(chrono_tz::UTC);
        let start = matcher::parse_channel_time(&ch.name, zone, now);
        // An event channel whose named start is long past is over (its name goes stale).
        if start.is_some_and(|t| t < now - EVENT_CHANNEL_STALE) {
            continue;
        }
        for m in ms {
            if covered.contains(&(ch.id, m.league.clone())) {
                continue;
            }
            out.push(MatchRow {
                channel_id: ch.id,
                start_utc: start,
                end_utc: None,
                m,
                source: "channel",
            });
        }
    }
    out
}

/// An event channel named for a start more than this long ago is treated as over.
const EVENT_CHANNEL_STALE: i64 = 4 * 3600;

/// Guides that never mark live airings repeat the same listing (a practice session at 21:30,
/// 01:00 and 07:00). Only the first airing in the window stays "likely live"; later airings of
/// the same title on the same channel are replays.
fn demote_repeats(programmes: &[ProgrammeRow], out: &mut [MatchRow]) {
    let text: HashMap<(i64, i64), String> = programmes
        .iter()
        .map(|p| {
            let t = format!(
                "{} {}",
                p.title.as_deref().unwrap_or(""),
                p.subtitle.as_deref().unwrap_or("")
            );
            (
                (p.channel_id, p.start_utc),
                matcher::normalize(&t).join(" "),
            )
        })
        .collect();
    let mut order: Vec<usize> = (0..out.len()).collect();
    order.sort_by_key(|&i| out[i].start_utc);
    let mut seen: HashSet<(i64, String, String)> = HashSet::new();
    for i in order {
        let r = &mut out[i];
        if !r.m.live_inferred {
            continue;
        }
        let Some(start) = r.start_utc else { continue };
        let Some(t) = text.get(&(r.channel_id, start)) else {
            continue;
        };
        if !seen.insert((r.channel_id, r.m.league.clone(), t.clone())) {
            r.m.live = false;
            r.m.replay = true;
        }
    }
}

/// A later airing of a team game counts as a re-air when it starts this long after the first one.
const TEAM_REAIR_WINDOW: i64 = 18 * 3600;

/// Guides mark the odd re-air of a game live (a midnight repeat of the afternoon match). Two
/// teams meet once a day, so a "live" airing of the same matchup that starts after every copy of
/// the first airing has ended, within 18 hours, is a replay. Baseball is left alone:
/// doubleheaders are two live games between the same teams on one day.
fn demote_team_reairs(cat: &Catalog, out: &mut [MatchRow]) {
    let mut games: HashMap<(String, Vec<String>), Vec<usize>> = HashMap::new();
    for (i, r) in out.iter().enumerate() {
        let team_league = cat
            .league(&r.m.league)
            .is_some_and(|l| l.kind == Kind::Team && l.sport != "baseball");
        if !team_league || !r.m.live || r.m.teams.len() < 2 || r.source != "programme" {
            continue;
        }
        let mut teams = r.m.teams.clone();
        teams.sort();
        games
            .entry((r.m.league.clone(), teams))
            .or_default()
            .push(i);
    }
    for idx in games.into_values() {
        let mut airings: Vec<(i64, i64, usize)> = idx
            .into_iter()
            .filter_map(|i| Some((out[i].start_utc?, out[i].end_utc?, i)))
            .collect();
        airings.sort_unstable();
        let Some(&(first_start, mut first_end, _)) = airings.first() else {
            continue;
        };
        for (start, end, i) in airings {
            if start < first_end {
                // Another copy of the first airing (overlapping): it can run longer.
                first_end = first_end.max(end);
            } else if start - first_start < TEAM_REAIR_WINDOW {
                out[i].m.live = false;
                out[i].m.replay = true;
            }
        }
    }
}

/// A stored match.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct StoredMatch {
    pub channel_id: i64,
    pub start_utc: Option<i64>,
    pub end_utc: Option<i64>,
    pub league: String,
    pub teams: String,
    pub session: Option<String>,
    pub live: bool,
    pub live_inferred: bool,
    pub score: i64,
    pub source: String,
}

/// Stored matches for the follow document: programme matches overlapping `[start, end)` plus
/// every channel-name match, honoring `includeReplays` and per-series session filters. Sorted
/// live first, then by start (channel-name matches without a time sort as "now").
pub async fn list(
    store: &Store,
    cat: &Catalog,
    follow: &SportsFollowDto,
    start: i64,
    end: i64,
    now: i64,
) -> Result<Vec<StoredMatch>> {
    let mut rows: Vec<StoredMatch> = sqlx::query_as(
        "SELECT channel_id, start_utc, end_utc, league, teams, session, live, live_inferred,
                score, source
         FROM sports_match
         WHERE source = 'channel' OR (start_utc < ?2 AND end_utc > ?1)",
    )
    .bind(start)
    .bind(end)
    .fetch_all(store.pool())
    .await?;
    rows.retain(|r| {
        let Some(l) = cat.league(&r.league) else {
            return false;
        };
        if !follow.include_replays && !r.live {
            return false;
        }
        match l.kind {
            Kind::Team => follow.leagues.contains_key(&l.key),
            Kind::Motorsport => follow.series.get(&l.key).is_some_and(|s| {
                r.session
                    .as_deref()
                    .is_some_and(|x| s.sessions.iter().any(|f| f == x))
            }),
        }
    });
    rows.sort_by(|a, b| {
        b.live
            .cmp(&a.live)
            .then(a.start_utc.unwrap_or(now).cmp(&b.start_utc.unwrap_or(now)))
            .then(b.score.cmp(&a.score))
            .then(a.channel_id.cmp(&b.channel_id))
            .then(a.league.cmp(&b.league))
    });
    Ok(rows)
}

/// Programme rows for `(channel_id, start_utc)` keys, in one query.
pub async fn programmes_for(
    store: &Store,
    keys: &[(i64, i64)],
) -> Result<HashMap<(i64, i64), ProgrammeRow>> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }
    let json = serde_json::to_string(keys)?;
    let rows: Vec<ProgrammeRow> = sqlx::query_as(
        "SELECT p.channel_id, p.start_utc, p.end_utc, p.ext_id, p.title, p.subtitle,
                p.description, p.season, p.episode, p.flags, p.icon
         FROM json_each(?1) j
         JOIN programme p ON p.channel_id = json_extract(j.value, '$[0]')
                         AND p.start_utc = json_extract(j.value, '$[1]')",
    )
    .bind(json)
    .fetch_all(store.pool())
    .await?;
    Ok(rows
        .into_iter()
        .map(|p| ((p.channel_id, p.start_utc), p))
        .collect())
}

#[cfg(test)]
mod tests;
