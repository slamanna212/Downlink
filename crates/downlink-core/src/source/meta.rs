//! Extra title metadata a source can report beyond the catalog row: guide credits and ratings
//! (Dispatcharr's programme detail), VOD cast and release data (`provider-info`). Everything is
//! optional; the service merges it with TMDB, and the source's values always win.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::dispatcharr::dto::{flex_i64, flex_string};

/// A credited person. `role` is the character for actors.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Credit {
    pub name: String,
    pub role: Option<String>,
    /// Absolute image URL (TMDB only; sources rarely send one).
    pub photo: Option<String>,
    pub guest: bool,
}

/// What the server's programme detail adds to a guide row.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProgrammeExtra {
    pub categories: Vec<String>,
    /// Content rating as broadcast (`TV-14`), with its system (`VCHIP`, `MPAA`) when known.
    pub rating: Option<String>,
    pub rating_system: Option<String>,
    /// First star rating, normalised to 0..10.
    pub score: Option<f64>,
    pub actors: Vec<Credit>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    pub producers: Vec<String>,
    pub presenters: Vec<String>,
    /// `HD`, `UHD`… as the guide says.
    pub video_quality: Option<String>,
    pub aspect_ratio: Option<String>,
    pub stereo: Option<String>,
    pub previously_shown: bool,
    pub country: Option<String>,
    pub language: Option<String>,
    /// XMLTV `<date>`: production year or date (`2019`, `20190412`).
    pub production_date: Option<String>,
    /// Original air date (`YYYY-MM-DD` when parseable, else as sent).
    pub original_air_date: Option<String>,
    pub runtime_mins: Option<i64>,
    pub content_advisory: Vec<String>,
    pub imdb_id: Option<String>,
    pub tmdb_id: Option<String>,
    /// `true` when the guide's TMDB id names a movie (`movie/123`).
    pub tmdb_is_movie: Option<bool>,
    pub tvdb_id: Option<String>,
    /// Absolute artwork URLs, best first (poster proxy, then icon, then images).
    pub images: Vec<String>,
}

/// What a VOD detail call adds to a movie or series row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VodExtra {
    pub cast: Vec<String>,
    pub directors: Vec<String>,
    pub country: Option<String>,
    /// `YYYY-MM-DD` when the source sends a date.
    pub release_date: Option<String>,
    /// Age rating / certification as the provider says (`PG-13`, `16`).
    pub certification: Option<String>,
    pub original_title: Option<String>,
    /// YouTube video id of a trailer.
    pub trailer: Option<String>,
}

impl VodExtra {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Split a provider's people list (`"A, B, C"` or a JSON array) into names.
pub fn people(v: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| {
                x.get("name")
                    .and_then(flex_string)
                    .or_else(|| flex_string(x))
            })
            .collect(),
        Some(other) => flex_string(other)
            .map(|s| s.split(',').map(|p| p.trim().to_owned()).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    out.retain(|n| !n.is_empty());
    out.dedup();
    out
}

/// `YYYY-MM-DD` from `2019-04-12`, `20190412`, `2019-04-12T…`; anything else is kept only when it
/// is a bare year.
pub fn iso_date(raw: &str) -> Option<String> {
    let t = raw.trim();
    let digits: String = t.chars().filter(char::is_ascii_digit).take(8).collect();
    let b = t.as_bytes();
    if b.len() >= 10 && b[4] == b'-' && b[7] == b'-' {
        return Some(t[..10].to_owned());
    }
    if digits.len() == 8 && t.len() >= 8 && t[..8].bytes().all(|c| c.is_ascii_digit()) {
        return Some(format!(
            "{}-{}-{}",
            &digits[..4],
            &digits[4..6],
            &digits[6..8]
        ));
    }
    None
}

/// The year in a date-ish string (`2019`, `20190412`, `2019-04-12`).
pub fn year_of(raw: &str) -> Option<i64> {
    let t = raw.trim();
    let y: i64 = t.get(..4)?.parse().ok()?;
    (1870..=2100).contains(&y).then_some(y)
}

/// A star rating such as `7.5/10`, `3/4`, `8` → 0..10.
pub fn score_of(raw: &str) -> Option<f64> {
    let t = raw.trim();
    let (num, den) = match t.split_once('/') {
        Some((n, d)) => (n.trim().parse::<f64>().ok()?, d.trim().parse::<f64>().ok()?),
        None => (t.parse::<f64>().ok()?, 10.0),
    };
    if !(num.is_finite() && den.is_finite()) || den <= 0.0 || num <= 0.0 || num > den {
        return None;
    }
    Some((num / den * 100.0).round() / 10.0)
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(flex_string)
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn opt(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(flex_string).filter(|s| !s.is_empty())
}

/// A guide TMDB id: `1234`, `tv/1234`, `movie/1234`, `series/1234`.
fn guide_tmdb(raw: &str) -> Option<(String, Option<bool>)> {
    let t = raw.trim().trim_end_matches('/');
    let (kind, id) = match t.rsplit_once('/') {
        Some((k, id)) => (Some(k.to_ascii_lowercase()), id),
        None => (None, t),
    };
    let id = super::vod::clean_tmdb(Some(id.to_owned()))?;
    let movie = kind.map(|k| k.ends_with("movie"));
    Some((id, movie))
}

/// Parse `GET /api/epg/programs/{id}/` (Dispatcharr 0.34+). `server_url` maps relative or
/// same-host artwork onto the configured base.
pub fn programme_extra_from(
    v: &Value,
    server_url: impl Fn(&str) -> Option<String>,
) -> ProgrammeExtra {
    let credits = v.get("credits").cloned().unwrap_or(Value::Null);
    let actors = match credits.get("actors") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| {
                if x.is_object() {
                    Some(Credit {
                        name: opt(x, "name")?,
                        role: opt(x, "role"),
                        photo: None,
                        guest: x.get("guest").and_then(Value::as_bool).unwrap_or(false),
                    })
                } else {
                    flex_string(x).filter(|s| !s.is_empty()).map(|name| Credit {
                        name,
                        ..Credit::default()
                    })
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    let score = match v.get("star_ratings") {
        Some(Value::Array(a)) => a.iter().find_map(|r| {
            r.get("value")
                .and_then(flex_string)
                .and_then(|s| score_of(&s))
        }),
        _ => None,
    };
    let runtime_mins = v.get("runtime").and_then(flex_i64).and_then(|n| {
        match opt(v, "runtime_units")
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("seconds") => Some(n / 60),
            Some("hours") => Some(n * 60),
            _ => Some(n),
        }
        .filter(|m| *m > 0)
    });
    let content_advisory = match v.get("content_advisory") {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| flex_string(x).or_else(|| x.get("text").and_then(flex_string)))
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    };
    let mut images: Vec<String> = Vec::new();
    for raw in [opt(v, "poster_url"), opt(v, "icon")].into_iter().flatten() {
        if let Some(u) = server_url(&raw) {
            images.push(u);
        }
    }
    if let Some(Value::Array(a)) = v.get("images") {
        for i in a {
            if let Some(u) = i
                .get("url")
                .and_then(flex_string)
                .or_else(|| flex_string(i))
                .and_then(|u| server_url(&u))
            {
                images.push(u);
            }
        }
    }
    images.dedup();
    let (tmdb_id, tmdb_is_movie) = match opt(v, "tmdb_id").as_deref().and_then(guide_tmdb) {
        Some((id, m)) => (Some(id), m),
        None => (None, None),
    };
    ProgrammeExtra {
        categories: strings(v.get("categories")),
        rating: opt(v, "rating"),
        rating_system: opt(v, "rating_system"),
        score,
        actors,
        directors: strings(credits.get("directors")),
        writers: strings(credits.get("writers")),
        producers: strings(credits.get("producers")),
        presenters: strings(credits.get("presenters")),
        video_quality: opt(v, "video_quality"),
        aspect_ratio: opt(v, "aspect_ratio"),
        stereo: opt(v, "stereo"),
        previously_shown: v
            .get("is_previously_shown")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        country: opt(v, "country"),
        language: opt(v, "language"),
        production_date: opt(v, "production_date"),
        original_air_date: opt(v, "original_air_date").map(|d| iso_date(&d).unwrap_or(d)),
        runtime_mins,
        content_advisory,
        imdb_id: super::vod::clean_imdb(opt(v, "imdb_id")),
        tmdb_id,
        tmdb_is_movie,
        tvdb_id: opt(v, "tvdb_id"),
        images,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn dates_scores_and_people() {
        assert_eq!(iso_date("20190412").as_deref(), Some("2019-04-12"));
        assert_eq!(
            iso_date("2019-04-12T10:00:00Z").as_deref(),
            Some("2019-04-12")
        );
        assert_eq!(iso_date("2019"), None);
        assert_eq!(year_of("20190412"), Some(2019));
        assert_eq!(year_of("abc"), None);
        assert_eq!(score_of("7.5/10"), Some(7.5));
        assert_eq!(score_of("3/4"), Some(7.5));
        assert_eq!(score_of("8"), Some(8.0));
        assert_eq!(score_of("0/10"), None);
        assert_eq!(score_of("11/10"), None);
        assert_eq!(
            people(Some(&json!("Ann Example, Bo Sample ,"))),
            vec!["Ann Example", "Bo Sample"]
        );
        assert_eq!(
            people(Some(&json!([{"name": "Cy Test"}, "Di Demo"]))),
            vec!["Cy Test", "Di Demo"]
        );
    }

    #[test]
    fn guide_tmdb_forms() {
        assert_eq!(guide_tmdb("1234"), Some(("1234".into(), None)));
        assert_eq!(guide_tmdb("movie/77"), Some(("77".into(), Some(true))));
        assert_eq!(guide_tmdb("tv/88/"), Some(("88".into(), Some(false))));
        assert_eq!(guide_tmdb("tv/x"), None);
    }

    #[test]
    fn parses_the_detail_fixture() {
        let raw =
            include_str!("../../../../tools/fixtures/dispatcharr/epg_program_detail_rich.json");
        let v: Value = serde_json::from_str(raw).unwrap();
        let x = programme_extra_from(&v, |u| Some(format!("https://srv.test{u}")));
        assert_eq!(x.categories, vec!["Drama", "Mystery"]);
        assert_eq!(x.rating.as_deref(), Some("TV-14"));
        assert_eq!(x.score, Some(8.0));
        assert_eq!(x.actors.len(), 2);
        assert_eq!(x.actors[0].role.as_deref(), Some("Detective Vale"));
        assert!(x.actors[1].guest);
        assert_eq!(x.directors, vec!["Rae Director"]);
        assert_eq!(x.original_air_date.as_deref(), Some("2024-03-02"));
        assert_eq!(x.runtime_mins, Some(44));
        assert_eq!(x.tmdb_id.as_deref(), Some("4242"));
        assert_eq!(x.tmdb_is_movie, Some(false));
        assert!(x.previously_shown);
        assert_eq!(
            x.images[0],
            "https://srv.test/api/epg/programs/9001/poster/?v=abc"
        );
    }
}
