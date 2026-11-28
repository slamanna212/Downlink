//! Saved command centers (docs/COMMAND_CENTER.md §6): named multiview walls the user built by hand.
//! Validation of what the UI sends, and the `command_center` store. Visibility of the tiles'
//! channels comes from `catalog/` (the service filters on load), never from here.

pub mod store;

use serde_json::Value;

use crate::error::{AppError, Result};
use crate::service::dto::CommandCenterSaveDto;

/// Name length in characters, after trimming.
pub const NAME_MAX: usize = 80;
/// Slots are 0 (main player) ..= 8.
pub const MAX_SLOT: i64 = 8;
pub const MAX_TILES: usize = 9;
/// Upper bound on the serialized rail document.
pub const RAIL_MAX_BYTES: usize = 64 * 1024;
pub const LAYOUT_STYLES: [&str; 2] = ["grid", "featured"];

/// A save request that passed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidSave {
    pub id: Option<i64>,
    pub name: String,
    pub layout_style: String,
    pub audible: i64,
    pub rail_json: String,
    /// (slot, channel id), slot order.
    pub tiles: Vec<(i64, i64)>,
}

/// Check a save request: trimmed name of 1..=80 characters, a known layout style, an audible slot
/// in range, a JSON object rail of at most 64 KiB, at most nine tiles on unique slots 0..=8.
pub fn validate(dto: &CommandCenterSaveDto) -> Result<ValidSave> {
    let name = dto.name.trim();
    let len = name.chars().count();
    if len == 0 {
        return Err(AppError::local("Give the command center a name."));
    }
    if len > NAME_MAX {
        return Err(AppError::local(format!(
            "Command center names can be up to {NAME_MAX} characters."
        )));
    }
    if !LAYOUT_STYLES.contains(&dto.layout_style.as_str()) {
        return Err(AppError::local("Unknown multiview layout."));
    }
    if !(0..=MAX_SLOT).contains(&dto.audible) {
        return Err(AppError::local("The audible tile is out of range."));
    }
    if !matches!(dto.rail, Value::Object(_)) {
        return Err(AppError::local(
            "The command center rail could not be saved.",
        ));
    }
    let rail_json = serde_json::to_string(&dto.rail)?;
    if rail_json.len() > RAIL_MAX_BYTES {
        return Err(AppError::local(
            "The command center rail is too large to save.",
        ));
    }
    if dto.tiles.len() > MAX_TILES {
        return Err(AppError::local(format!(
            "A command center holds up to {MAX_TILES} tiles."
        )));
    }
    let mut tiles: Vec<(i64, i64)> = Vec::with_capacity(dto.tiles.len());
    for t in &dto.tiles {
        if !(0..=MAX_SLOT).contains(&t.slot) {
            return Err(AppError::local("A tile is out of range."));
        }
        if tiles.iter().any(|(s, _)| *s == t.slot) {
            return Err(AppError::local("Two tiles share a slot."));
        }
        tiles.push((t.slot, t.channel_id));
    }
    tiles.sort_unstable();
    Ok(ValidSave {
        id: dto.id,
        name: name.to_owned(),
        layout_style: dto.layout_style.clone(),
        audible: dto.audible,
        rail_json,
        tiles,
    })
}

/// The stored rail as a JSON object; anything unreadable comes back as `{}`.
pub fn parse_rail(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Default::default()))
}

#[cfg(test)]
mod tests;
