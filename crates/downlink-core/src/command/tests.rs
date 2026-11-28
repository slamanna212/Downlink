//! Save-request validation.

use serde_json::json;

use super::*;
use crate::service::dto::CommandCenterTileSaveDto;

fn dto(v: serde_json::Value) -> CommandCenterSaveDto {
    serde_json::from_value(v).unwrap()
}

fn tiles(n: i64) -> Vec<serde_json::Value> {
    (0..n)
        .map(|s| json!({ "slot": s, "channelId": 100 + s }))
        .collect()
}

#[test]
fn accepts_and_normalizes_a_valid_request() {
    let v = validate(&dto(json!({
        "name": "  Election night  ",
        "layoutStyle": "featured",
        "audible": 2,
        "rail": { "feeds": [{ "id": 1 }] },
        "tiles": [{ "slot": 2, "channelId": 7 }, { "slot": 0, "channelId": 5 }]
    })))
    .unwrap();
    assert_eq!(v.id, None);
    assert_eq!(v.name, "Election night");
    assert_eq!(v.tiles, [(0, 5), (2, 7)]);
    assert_eq!(v.rail_json, r#"{"feeds":[{"id":1}]}"#);
}

#[test]
fn defaults_fill_missing_fields() {
    let v = validate(&dto(json!({ "name": "Wall" }))).unwrap();
    assert_eq!(
        (v.layout_style.as_str(), v.audible, v.rail_json.as_str()),
        ("grid", 0, "{}")
    );
    assert!(v.tiles.is_empty());
}

#[test]
fn rejects_bad_names_styles_rails_and_slots() {
    let base = || dto(json!({ "name": "Wall", "tiles": tiles(2) }));
    let check = |f: &dyn Fn(&mut CommandCenterSaveDto)| {
        let mut d = base();
        f(&mut d);
        validate(&d).is_err()
    };
    assert!(!check(&|_| {}));
    assert!(check(&|d| d.name = "   ".into()));
    assert!(check(&|d| d.name = "x".repeat(NAME_MAX + 1)));
    assert!(
        !check(&|d| d.name = "é".repeat(NAME_MAX)),
        "counts characters"
    );
    assert!(check(&|d| d.layout_style = "mosaic".into()));
    assert!(check(&|d| d.audible = 9));
    assert!(check(&|d| d.audible = -1));
    assert!(check(&|d| d.rail = json!([])));
    assert!(check(&|d| d.rail = json!("feeds")));
    assert!(check(
        &|d| d.rail = json!({ "pad": "x".repeat(RAIL_MAX_BYTES) })
    ));
    assert!(check(&|d| d.tiles[1].slot = 0), "duplicate slot");
    assert!(check(&|d| d.tiles[1].slot = 9));
    assert!(check(&|d| d.tiles[1].slot = -1));
    assert!(check(&|d| {
        d.tiles = (0..10)
            .map(|s| CommandCenterTileSaveDto {
                slot: s % 9,
                channel_id: s,
            })
            .collect()
    }));
    assert!(!check(&|d| {
        d.tiles = (0..9)
            .map(|s| CommandCenterTileSaveDto {
                slot: s,
                channel_id: s,
            })
            .collect()
    }));
}

#[test]
fn unreadable_rails_load_as_an_empty_object() {
    assert_eq!(parse_rail(r#"{"feeds":[]}"#), json!({ "feeds": [] }));
    assert_eq!(parse_rail("[1]"), json!({}));
    assert_eq!(parse_rail("not json"), json!({}));
}
