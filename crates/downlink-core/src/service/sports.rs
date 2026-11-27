//! Sports verbs (docs/SPORTS.md §4): catalog, follow settings, matches, and the rebuild that
//! runs after guide syncs and settings changes.

use std::collections::HashMap;
use std::sync::atomic::Ordering;

use tokio::sync::broadcast;

use super::{
    CatalogChange, CatalogUpdated, Event, ProgrammeDto, Service, SportsCatalogDto, SportsFollowDto,
    SportsLeagueDto, SportsMatchDto, SportsTeamDto,
};
use crate::catalog;
use crate::error::Result;
use crate::sports::{self, store};
use crate::store::Store;
use crate::sync::now_utc;

impl Service {
    pub fn sports_catalog(&self) -> Result<SportsCatalogDto> {
        let cat = sports::catalog()?;
        Ok(SportsCatalogDto {
            leagues: cat
                .leagues
                .iter()
                .map(|l| SportsLeagueDto {
                    key: l.key.clone(),
                    name: l.name.clone(),
                    kind: l.kind.as_str().to_owned(),
                    group: l.group.clone(),
                    teams: l
                        .teams
                        .iter()
                        .map(|t| SportsTeamDto {
                            key: t.key.clone(),
                            name: t.name.clone(),
                            short: t.short.clone(),
                            abbr: t.abbr.clone(),
                        })
                        .collect(),
                })
                .collect(),
            sessions: sports::catalog::SESSIONS
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        })
    }

    pub async fn sports_follow_get(&self) -> Result<SportsFollowDto> {
        store::load_follow(&self.store).await
    }

    /// Save the follow document, then rebuild the matches (emits `catalog://updated`, scope
    /// `sports`).
    pub async fn sports_follow_set(&self, follow: SportsFollowDto) -> Result<()> {
        let cat = sports::catalog()?;
        store::save_follow(&self.store, cat, &follow, now_utc()).await?;
        self.rebuild_sports().await?;
        Ok(())
    }

    /// Rebuild `sports_match` for now .. +7 days and wait for it. One rebuild at a time; scoring
    /// runs on a blocking thread so it never holds up playback.
    pub async fn rebuild_sports(&self) -> Result<store::RebuildReport> {
        let _one = self.sports_rebuild.lock().await;
        self.sports_dirty.store(false, Ordering::SeqCst);
        rebuild_and_emit(&self.store, &self.events).await
    }

    /// Background trigger (after syncs and guide refreshes): returns at once, so a guide refresh
    /// never waits on matching. Requests that land while a rebuild runs collapse into one more
    /// pass. Failures are logged, never surfaced.
    pub(super) async fn rebuild_sports_logged(&self) {
        self.sports_dirty.store(true, Ordering::SeqCst);
        let Ok(guard) = self.sports_rebuild.clone().try_lock_owned() else {
            return; // the running rebuild sees the flag and goes again
        };
        let store = self.store.clone();
        let events = self.events.clone();
        let dirty = self.sports_dirty.clone();
        tokio::spawn(async move {
            let _one = guard;
            while dirty.swap(false, Ordering::SeqCst) {
                if let Err(e) = rebuild_and_emit(&store, &events).await {
                    tracing::warn!(error = %e, "sports match rebuild failed");
                }
            }
        });
    }

    /// Followed matches: programmes overlapping `[start_utc, end_utc)` plus channel-name matches,
    /// honoring `includeReplays` and session filters; live first, then by start.
    pub async fn sports_matches(
        &self,
        start_utc: i64,
        end_utc: i64,
    ) -> Result<Vec<SportsMatchDto>> {
        let cat = sports::catalog()?;
        let follow = store::load_follow(&self.store).await?;
        let rows = store::list(&self.store, cat, &follow, start_utc, end_utc, now_utc()).await?;
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        // Visibility is re-applied here: the content filters may have changed since the rebuild.
        let mut ids: Vec<i64> = rows.iter().map(|r| r.channel_id).collect();
        ids.sort_unstable();
        ids.dedup();
        let filter = self.filter().await?;
        let channel_rows = catalog::channels_by_ids(&self.store, &filter, &ids).await?;
        let channels: HashMap<i64, super::ChannelDto> = self
            .to_channel_dtos(&channel_rows)
            .await?
            .into_iter()
            .map(|c| (c.id, c))
            .collect();
        let keys: Vec<(i64, i64)> = rows
            .iter()
            .filter(|r| r.source == "programme")
            .filter_map(|r| r.start_utc.map(|s| (r.channel_id, s)))
            .collect();
        let progs = store::programmes_for(&self.store, &keys).await?;
        let mut programme_dtos: Vec<ProgrammeDto> =
            progs.values().map(ProgrammeDto::from).collect();
        self.mark_scheduled(&mut programme_dtos).await?;
        let programme_dtos: HashMap<(i64, i64), ProgrammeDto> = programme_dtos
            .into_iter()
            .map(|p| ((p.channel_id, p.start_utc), p))
            .collect();

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let Some(channel) = channels.get(&r.channel_id) else {
                continue;
            };
            let Some(league) = cat.league(&r.league) else {
                continue;
            };
            let programme = if r.source == "programme" {
                let Some(p) = r
                    .start_utc
                    .and_then(|s| programme_dtos.get(&(r.channel_id, s)))
                else {
                    continue; // the guide row is gone since the rebuild
                };
                Some(p.clone())
            } else {
                None
            };
            let teams: Vec<String> = r
                .teams
                .split(',')
                .filter(|t| !t.is_empty())
                .map(str::to_owned)
                .collect();
            let followed_teams = follow
                .leagues
                .get(&r.league)
                .map(|lf| {
                    teams
                        .iter()
                        .filter(|t| lf.teams.contains(t))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let event_start_utc = if programme.is_none() {
                r.start_utc
            } else {
                None
            };
            out.push(SportsMatchDto {
                event_start_utc,
                channel: channel.clone(),
                programme,
                league: r.league,
                league_name: league.name.clone(),
                teams,
                followed_teams,
                session: r.session,
                live: r.live,
                live_inferred: r.live_inferred,
                source: r.source,
                score: r.score,
                alternates: Vec::new(),
            });
        }
        Ok(sports::group::group_events(out, now_utc()))
    }
}

/// One rebuild with the current content filter, then `catalog://updated` (scope `sports`).
async fn rebuild_and_emit(
    store: &Store,
    events: &broadcast::Sender<Event>,
) -> Result<store::RebuildReport> {
    let cat = sports::catalog()?;
    let filter = super::channel_filter(store).await?;
    let report = store::rebuild(store, cat, &filter, now_utc()).await?;
    tracing::debug!(
        candidates = report.candidates,
        matches = report.matches,
        "sports matches rebuilt"
    );
    let _ = events.send(Event::CatalogUpdated(CatalogUpdated {
        scope: CatalogChange::Sports,
        source_id: 0,
        channels: 0,
        groups: 0,
        programmes: 0,
    }));
    Ok(report)
}
