//! Multiview (PLAN §4.11): up to 9 live tiles. Slot 0 is the main player (its own glue, failover and
//! state events); slots 1.. are tiles created here, each with its own mpv instance and native surface
//! under the webview. Exactly one slot is audible.
//!
//! Rule 6: removing or replacing a tile, leaving multiview, sleep and quit stop that tile's stream.
//! Tiles always use the TS path (`Service::tile_plan`), so they never open a server session.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

use downlink_core::service::Service;
use downlink_core::{AppError, ErrorCode};
use downlink_player::{
    EventSink, FailureKind, LoadedMpv, MAX_SURFACES, MainThreadDispatch, PlayMode, Player,
    PlayerCommand, PlayerEvent, Profile, RenderBackend, SessionState, SurfaceBounds, VideoSurface,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::dto::phase_name;
use crate::surface::Surface;

/// Tiles on screen when hardware decode is not active, or on a small budget.
pub const BASE_TILES: u32 = 4;
/// Upper bound with working hardware decode and a budget that allows it.
pub const CAPABLE_TILES: u32 = 9;

/// One tile's state for the UI (`MultiviewTile` of types.ts). Slot 0 is reported from the main
/// player's channel; its playback state comes from `player://state`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TileDto {
    pub slot: u32,
    pub channel_id: i64,
    pub state: &'static str,
    /// `limit` | `codec` | `engine` | `stream` when failed.
    pub failure: Option<&'static str>,
}

/// `MultiviewState` of types.ts, emitted as `multiview://state`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MultiviewDto {
    pub tiles: Vec<TileDto>,
    /// The audible slot (0 = main player).
    pub audible: u32,
    /// Tiles this machine may show, main player included (4, or 9 with hwdec and budget).
    pub max_tiles: u32,
    /// Concurrent streams the account may hold, when the source reports a limit.
    pub stream_limit: Option<u32>,
}

/// Pure rule for the tile cap (PLAN §4.11): 9 only with hardware decode and a budget of at least 9,
/// never above what the platform's surfaces allow.
pub fn max_tiles(platform_max: u32, hwdec_active: bool, stream_limit: Option<u32>) -> u32 {
    let cap = if hwdec_active && stream_limit.is_none_or(|l| l >= CAPABLE_TILES) {
        CAPABLE_TILES
    } else {
        BASE_TILES
    };
    let budget = stream_limit.map_or(cap, |l| l.max(1));
    cap.min(budget).min(platform_max)
}

/// `hwdec-current` names a real decoder.
pub fn hwdec_active(current: Option<&str>) -> bool {
    current.is_some_and(|c| !c.is_empty() && c != "no")
}

fn failure_name(k: FailureKind) -> &'static str {
    match k {
        FailureKind::Limit => "limit",
        FailureKind::Codec => "codec",
        FailureKind::Engine => "engine",
        FailureKind::Stream => "stream",
    }
}

struct Tile {
    player: Arc<Player>,
    surface: Arc<Mutex<Surface>>,
    hls: Option<crate::hls_proxy::HlsProxy>,
    generation: u64,
}

#[derive(Default)]
struct Status {
    channel_id: i64,
    state: Option<SessionState>,
    failure: Option<FailureKind>,
}

#[derive(Default)]
struct Shared {
    status: BTreeMap<u32, Status>,
    audible: u32,
    /// The main player was muted by multiview (audio moved to a tile) and is unmuted on exit.
    main_muted_by_us: bool,
}

pub struct Multiview {
    app: AppHandle,
    service: Arc<Service>,
    loaded: LoadedMpv,
    dispatch: MainThreadDispatch,
    main: Arc<Player>,
    tiles: tokio::sync::Mutex<HashMap<u32, Tile>>,
    shared: Arc<Mutex<Shared>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    match m.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    }
}

fn mv_err(msg: impl AsRef<str>) -> AppError {
    AppError::new(ErrorCode::Media, msg)
}

impl Multiview {
    pub fn new(
        app: AppHandle,
        service: Arc<Service>,
        loaded: LoadedMpv,
        dispatch: MainThreadDispatch,
        main: Arc<Player>,
    ) -> Arc<Self> {
        Arc::new(Self {
            app,
            service,
            loaded,
            dispatch,
            main,
            tiles: tokio::sync::Mutex::new(HashMap::new()),
            shared: Arc::new(Mutex::new(Shared::default())),
        })
    }

    async fn stream_limit(&self) -> Option<u32> {
        let rows = self.service.store().list_sources().await.ok()?;
        rows.iter()
            .filter_map(|r| r.stream_limit)
            .filter_map(|n| u32::try_from(n).ok())
            .filter(|n| *n > 0)
            .min()
    }

    async fn max_tiles_now(&self) -> (u32, Option<u32>) {
        let limit = self.stream_limit().await;
        let hw = hwdec_active(self.main.snapshot().hwdec_current.as_deref());
        (max_tiles(MAX_SURFACES, hw, limit), limit)
    }

    pub async fn state(&self, main_channel: Option<i64>) -> MultiviewDto {
        let (max, limit) = self.max_tiles_now().await;
        dto_of(&lock(&self.shared), main_channel, max, limit)
    }

    async fn emit(&self, main_channel: Option<i64>) {
        let dto = self.state(main_channel).await;
        let _ = self.app.emit("multiview://state", dto);
    }

    fn sink(&self, slot: u32) -> EventSink {
        let shared = self.shared.clone();
        let app = self.app.clone();
        Arc::new(move |e| match e {
            PlayerEvent::State(snap) => {
                let changed = {
                    let mut g = lock(&shared);
                    // A removed tile's last events (its own stop) must not bring it back.
                    let Some(st) = g.status.get_mut(&slot) else {
                        return;
                    };
                    let failure = (snap.state == Some(SessionState::Failed))
                        .then(|| snap.failure.as_ref().map(|f| f.kind))
                        .flatten();
                    let changed = st.state != snap.state || st.failure != failure;
                    st.state = snap.state;
                    st.failure = failure;
                    changed
                };
                if changed {
                    let _ = app.emit("multiview://tile", slot);
                }
            }
            PlayerEvent::Log { text, .. } => tracing::debug!(target: "mpv", "tile {slot}: {text}"),
            _ => {}
        })
    }

    /// Put `channel_id` in `slot` (1..max), creating the tile if needed. A play (rule 4).
    pub async fn set_tile(
        &self,
        slot: u32,
        channel_id: i64,
        main_channel: Option<i64>,
    ) -> Result<(), AppError> {
        let (max, _) = self.max_tiles_now().await;
        if slot == 0 || slot >= max {
            return Err(AppError::local("That multiview slot is not available."));
        }
        let mut tiles = self.tiles.lock().await;
        if !tiles.contains_key(&slot) && 1 + tiles.len() as u32 >= max {
            return Err(AppError::local("Multiview is full."));
        }
        if let std::collections::hash_map::Entry::Vacant(e) = tiles.entry(slot) {
            e.insert(self.create_tile(slot).await?);
        }
        let dense = 1 + tiles.len() as u32 > BASE_TILES;
        let Some(tile) = tiles.get_mut(&slot) else {
            return Err(mv_err("The tile could not be created."));
        };
        tile.generation += 1;
        let generation = tile.generation;
        tile.hls = None;
        {
            let mut g = lock(&self.shared);
            let audible = g.audible == slot;
            let st = g.status.entry(slot).or_default();
            st.channel_id = channel_id;
            st.state = Some(SessionState::Resolving);
            st.failure = None;
            let _ = tile.player.command(PlayerCommand::Mute { muted: !audible });
        }
        let player = tile.player.clone();
        drop(tiles);
        player.begin_resolving();
        let prepared = async {
            let mut plan = self.service.tile_plan(channel_id).await?;
            let proxy = crate::hls_proxy::prepare(&mut plan, false, Some(self.app.clone())).await?;
            Ok::<_, AppError>((plan, proxy))
        }
        .await;
        let mut tiles = self.tiles.lock().await;
        let Some(tile) = tiles
            .get_mut(&slot)
            .filter(|tile| tile.generation == generation && Arc::ptr_eq(&tile.player, &player))
        else {
            return Ok(());
        };
        let (plan, proxy) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                player.mark_failed(error.message.clone());
                drop(tiles);
                self.emit(main_channel).await;
                return Err(error);
            }
        };
        tile.hls = proxy;
        let mut req = crate::player_glue::plan_request(&plan, PlayMode::Live, false);
        req.profile = if dense {
            Profile::MultiviewDense
        } else {
            Profile::Multiview
        };
        let r = player
            .play(req)
            .map(|_| ())
            .map_err(|e| mv_err(e.to_string()));
        drop(tiles);
        self.emit(main_channel).await;
        r
    }

    async fn create_tile(&self, slot: u32) -> Result<Tile, AppError> {
        let loaded = self.loaded.clone();
        let prefs = self.main.preferences();
        let sink = self.sink(slot);
        let player = tokio::task::spawn_blocking(move || {
            Player::new(&loaded, prefs, RenderBackend::platform_default(), sink)
        })
        .await
        .map_err(|e| mv_err(e.to_string()))?
        .map_err(|e| mv_err(e.to_string()))?;
        let player = Arc::new(player);
        let engine = player.engine();
        let dispatch = self.dispatch.clone();
        let surface = crate::boot::on_main(&self.app, move |h| {
            crate::surface::attach_slot(h, engine, dispatch, slot)
        })
        .await
        .ok_or_else(|| mv_err("The window is not available."))?
        .map_err(mv_err)?;
        let surface = Arc::new(Mutex::new(surface));
        let for_rebind = surface.clone();
        player.on_engine_replaced(move |engine| lock(&for_rebind).rebind(engine));
        Ok(Tile {
            player,
            surface,
            hls: None,
            generation: 0,
        })
    }

    /// Stop the tile's stream, free its surface (main thread), then its engine.
    async fn destroy(&self, tile: Tile) {
        if let Err(e) = tile.player.stop() {
            tracing::debug!("tile stop: {e}");
        }
        let surface = tile.surface.clone();
        let _ = crate::boot::on_main(&self.app, move |_| lock(&surface).detach()).await;
        drop(tile);
    }

    pub async fn remove_tile(&self, slot: u32, main_channel: Option<i64>) {
        let tile = self.tiles.lock().await.remove(&slot);
        {
            let mut g = lock(&self.shared);
            g.status.remove(&slot);
            if g.audible == slot {
                g.audible = 0;
                if g.main_muted_by_us {
                    g.main_muted_by_us = false;
                    let _ = self.main.command(PlayerCommand::Mute { muted: false });
                }
            }
        }
        if let Some(t) = tile {
            self.destroy(t).await;
        }
        self.emit(main_channel).await;
    }

    /// Make `slot` the only audible one (0 = main player).
    pub async fn set_audio(&self, slot: u32, main_channel: Option<i64>) {
        let tiles = self.tiles.lock().await;
        if slot != 0 && !tiles.contains_key(&slot) {
            return;
        }
        {
            let mut g = lock(&self.shared);
            g.audible = slot;
            for (s, t) in tiles.iter() {
                let _ = t.player.command(PlayerCommand::Mute { muted: *s != slot });
            }
            let main_muted = slot != 0;
            if main_muted != g.main_muted_by_us {
                g.main_muted_by_us = main_muted;
                let _ = self.main.command(PlayerCommand::Mute { muted: main_muted });
            }
        }
        drop(tiles);
        self.emit(main_channel).await;
    }

    pub async fn set_bounds(&self, slot: u32, bounds: SurfaceBounds, visible: bool) {
        if let Some(t) = self.tiles.lock().await.get(&slot) {
            let s = lock(&t.surface);
            s.set_bounds(bounds);
            s.set_visible(visible);
        }
    }

    pub async fn tile_channel(&self, slot: u32) -> Option<i64> {
        lock(&self.shared).status.get(&slot).map(|s| s.channel_id)
    }

    /// Dev driver: (slot, position, dropped frames) per tile.
    #[cfg(feature = "e2e")]
    pub async fn positions(&self) -> Vec<(u32, Option<f64>, Option<i64>, u64)> {
        let tiles = self.tiles.lock().await;
        let mut v: Vec<_> = tiles
            .iter()
            .map(|(s, t)| {
                let snap = t.player.snapshot();
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                let frames = lock(&t.surface).render_count();
                #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                let frames = 0;
                (*s, snap.position_secs, snap.dropped_frames, frames)
            })
            .collect();
        v.sort_by_key(|x| x.0);
        v
    }

    /// Leave multiview (and sleep / quit): every tile's stream stops, audio returns to the main player.
    pub async fn clear(&self) {
        let tiles: Vec<Tile> = self.tiles.lock().await.drain().map(|(_, t)| t).collect();
        let unmute = {
            let mut g = lock(&self.shared);
            g.status.clear();
            g.audible = 0;
            std::mem::take(&mut g.main_muted_by_us)
        };
        if unmute {
            let _ = self.main.command(PlayerCommand::Mute { muted: false });
        }
        for t in tiles {
            self.destroy(t).await;
        }
        let _ = self.app.emit("multiview://state", self.state(None).await);
    }
}

fn dto_of(g: &Shared, main_channel: Option<i64>, max: u32, limit: Option<u32>) -> MultiviewDto {
    let mut tiles: Vec<TileDto> = Vec::new();
    if let Some(c) = main_channel {
        tiles.push(TileDto {
            slot: 0,
            channel_id: c,
            state: "Playing",
            failure: None,
        });
    }
    tiles.extend(g.status.iter().map(|(slot, s)| TileDto {
        slot: *slot,
        channel_id: s.channel_id,
        state: phase_name(s.state.unwrap_or(SessionState::Idle)),
        failure: s.failure.map(failure_name),
    }));
    MultiviewDto {
        tiles,
        audible: g.audible,
        max_tiles: max,
        stream_limit: limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_cap_rules() {
        // No hwdec: 4.
        assert_eq!(max_tiles(9, false, None), 4);
        // hwdec and no known limit: 9.
        assert_eq!(max_tiles(9, true, None), 9);
        // hwdec but the account holds 6 streams: 4 (3x3 needs a budget of 9), and never above the budget.
        assert_eq!(max_tiles(9, true, Some(6)), 4);
        assert_eq!(max_tiles(9, false, Some(2)), 2);
        assert_eq!(max_tiles(9, true, Some(12)), 9);
        // Platform with a single surface (macOS today).
        assert_eq!(max_tiles(1, true, None), 1);
    }

    #[test]
    fn hwdec_detection() {
        assert!(hwdec_active(Some("vaapi")));
        assert!(!hwdec_active(Some("no")));
        assert!(!hwdec_active(None));
    }

    #[test]
    fn dto_lists_main_then_tiles() {
        let mut g = Shared::default();
        g.status.insert(
            2,
            Status {
                channel_id: 7,
                state: Some(SessionState::Failed),
                failure: Some(FailureKind::Limit),
            },
        );
        g.audible = 2;
        let d = dto_of(&g, Some(3), 4, Some(4));
        assert_eq!(d.tiles.len(), 2);
        assert_eq!((d.tiles[0].slot, d.tiles[0].channel_id), (0, 3));
        assert_eq!(d.tiles[1].failure, Some("limit"));
        assert_eq!(d.audible, 2);
    }
}
