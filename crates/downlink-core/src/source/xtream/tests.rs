use super::*;

#[test]
fn rating_prefers_the_ten_point_score() {
    assert_eq!(
        pick_rating(Some("7.4"), Some("3.7")).as_deref(),
        Some("7.4")
    );
}

#[test]
fn rating_falls_back_to_the_five_point_score() {
    assert_eq!(pick_rating(Some("0"), Some("3.7")).as_deref(), Some("7.4"));
    assert_eq!(pick_rating(None, Some("4")).as_deref(), Some("8.0"));
    assert_eq!(pick_rating(Some(""), Some(" 2.5 ")).as_deref(), Some("5.0"));
}

#[test]
fn rating_drops_missing_zero_and_out_of_scale() {
    assert_eq!(pick_rating(None, None), None);
    assert_eq!(pick_rating(Some("0"), Some("0")), None);
    assert_eq!(pick_rating(Some("0.0"), Some("")), None);
    assert_eq!(pick_rating(None, Some("9")), None);
}

#[test]
fn vod_dtos_read_string_and_numeric_ratings() {
    let m: VodStreamDto = serde_json::from_value(serde_json::json!({
        "stream_id": 1, "name": "Synthetic", "rating": 7.5, "rating_5based": 3.75
    }))
    .unwrap();
    assert_eq!(
        pick_rating(m.rating.as_deref(), m.rating_5based.as_deref()).as_deref(),
        Some("7.5")
    );
    let s: SeriesDto = serde_json::from_value(serde_json::json!({
        "series_id": "2", "name": "Synthetic", "rating": "", "rating_5based": "4.2"
    }))
    .unwrap();
    assert_eq!(
        pick_rating(s.rating.as_deref(), s.rating_5based.as_deref()).as_deref(),
        Some("8.4")
    );
}
