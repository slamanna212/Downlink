//! Saved command center verbs (docs/COMMAND_CENTER.md §6): list, get, save, delete. Tiles carry
//! full channel DTOs; a tile whose channel is hidden (`catalog/`) or gone is left out on load.

use std::collections::{HashMap, HashSet};

use super::{
    ChannelDto, CommandCenterDto, CommandCenterSaveDto, CommandCenterSummaryDto,
    CommandCenterTileDto, Service,
};
use crate::catalog;
use crate::command::{self, store};
use crate::error::{AppError, Result};
use crate::sync::now_utc;

/// Channels shown on a summary card.
const PREVIEW: usize = 4;

impl Service {
    /// Visible channel ids among `ids`.
    async fn visible_channel_ids(&self, ids: &[i64]) -> Result<HashSet<i64>> {
        let f = self.filter().await?;
        Ok(catalog::channels_by_ids(&self.store, &f, ids)
            .await?
            .into_iter()
            .map(|r| r.id)
            .collect())
    }

    pub async fn command_centers_list(&self) -> Result<Vec<CommandCenterSummaryDto>> {
        let centers = store::list(&self.store).await?;
        let tiles = store::tiles(&self.store, None).await?;
        let ids: Vec<i64> = tiles
            .iter()
            .map(|t| t.channel_id)
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let visible = self.visible_channel_ids(&ids).await?;
        let mut by_center: HashMap<i64, Vec<i64>> = HashMap::new();
        for t in tiles.iter().filter(|t| visible.contains(&t.channel_id)) {
            by_center.entry(t.center_id).or_default().push(t.channel_id);
        }
        Ok(centers
            .into_iter()
            .map(|c| {
                let chans = by_center.remove(&c.id).unwrap_or_default();
                CommandCenterSummaryDto {
                    id: c.id,
                    name: c.name,
                    tile_count: i64::try_from(chans.len()).unwrap_or(i64::MAX),
                    channel_ids: chans.into_iter().take(PREVIEW).collect(),
                    updated_utc: c.updated_utc,
                }
            })
            .collect())
    }

    pub async fn command_center_get(&self, id: i64) -> Result<CommandCenterDto> {
        let Some(c) = store::get(&self.store, id).await? else {
            return Err(AppError::local("That command center no longer exists."));
        };
        let tiles = store::tiles(&self.store, Some(id)).await?;
        let ids: Vec<i64> = tiles.iter().map(|t| t.channel_id).collect();
        let f = self.filter().await?;
        let rows = catalog::channels_by_ids(&self.store, &f, &ids).await?;
        let dtos: HashMap<i64, ChannelDto> = self
            .to_channel_dtos(&rows)
            .await?
            .into_iter()
            .map(|d| (d.id, d))
            .collect();
        Ok(CommandCenterDto {
            id: c.id,
            name: c.name,
            layout_style: c.layout_style,
            audible: c.audible,
            rail: command::parse_rail(&c.rail_json),
            tiles: tiles
                .iter()
                .filter_map(|t| {
                    dtos.get(&t.channel_id).map(|ch| CommandCenterTileDto {
                        slot: t.slot,
                        channel: ch.clone(),
                    })
                })
                .collect(),
            created_utc: c.created_utc,
            updated_utc: c.updated_utc,
        })
    }

    /// Create (no id) or replace a center. Returns its id.
    pub async fn command_center_save(&self, dto: CommandCenterSaveDto) -> Result<i64> {
        let v = command::validate(&dto)?;
        store::save(&self.store, &v, now_utc()).await
    }

    pub async fn command_center_delete(&self, id: i64) -> Result<()> {
        store::delete(&self.store, id).await
    }
}
