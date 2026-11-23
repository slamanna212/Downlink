//! VOD and DVR model shared by sources (PLAN M4). Items carry the source's own ids; local row ids
//! are assigned when they are cached in SQLite (`store::vod`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VodKind {
    Movie,
    Series,
    Episode,
}

impl VodKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Series => "series",
            Self::Episode => "episode",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "movie" => Some(Self::Movie),
            "series" => Some(Self::Series),
            "episode" => Some(Self::Episode),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VodCategoryInfo {
    pub ext_id: String,
    pub name: String,
    pub count: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VodItem {
    pub kind: Option<VodKind>,
    pub ext_id: String,
    pub uuid: Option<String>,
    /// Episodes: the series' ext id.
    pub parent_ext: Option<String>,
    pub title: String,
    pub year: Option<i64>,
    /// As the server reports it (usually "7.5").
    pub rating: Option<String>,
    pub genre: Option<String>,
    pub plot: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub duration_secs: Option<i64>,
    /// Absolute image URLs (may carry credentials: sources template them before storing).
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    pub category_ext: Option<String>,
    /// File extension for URL building (Xtream).
    pub container: Option<String>,
    /// TMDB id as the source reports it (digits), for media-server matching.
    pub tmdb_id: Option<String>,
    /// IMDb id (`tt…`), for media-server matching.
    pub imdb_id: Option<String>,
    /// Cast, director, release date… when the source reports them.
    pub extra: Option<super::VodExtra>,
}

/// A TMDB id as sources send it (`"1616400"`, `1616400`, `"0"`, `""`): digits only, not zero.
pub fn clean_tmdb(raw: Option<String>) -> Option<String> {
    let t = raw?.trim().to_owned();
    (!t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) && t.bytes().any(|b| b != b'0'))
        .then_some(t)
}

/// An IMDb id (`tt0111161`); anything else is dropped.
pub fn clean_imdb(raw: Option<String>) -> Option<String> {
    let t = raw?.trim().to_ascii_lowercase();
    (t.len() > 2 && t.starts_with("tt") && t[2..].bytes().all(|b| b.is_ascii_digit())).then_some(t)
}

#[derive(Debug, Clone, Default)]
pub struct VodListQuery {
    pub category_ext: Option<String>,
    pub search: Option<String>,
    pub offset: i64,
    pub limit: i64,
}

#[derive(Debug, Clone, Default)]
pub struct VodListPage {
    pub items: Vec<VodItem>,
    /// Total matching items across pages, when the source says.
    pub total: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_ids_are_cleaned() {
        assert_eq!(
            clean_tmdb(Some(" 1616400 ".into())).as_deref(),
            Some("1616400")
        );
        assert_eq!(clean_tmdb(Some("0".into())), None);
        assert_eq!(clean_tmdb(Some(String::new())), None);
        assert_eq!(clean_tmdb(Some("tt123".into())), None);
        assert_eq!(clean_tmdb(None), None);
        assert_eq!(
            clean_imdb(Some("TT0111161".into())).as_deref(),
            Some("tt0111161")
        );
        assert_eq!(clean_imdb(Some("tt".into())), None);
        assert_eq!(clean_imdb(Some("12345".into())), None);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecStatus {
    #[default]
    Scheduled,
    Recording,
    Completed,
    Failed,
}

impl RecStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scheduled => "scheduled",
            Self::Recording => "recording",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecordingInfo {
    pub ext_id: String,
    /// The source's channel id (`channel.ext_id`).
    pub channel_ext_id: String,
    pub title: String,
    pub start_utc: i64,
    pub end_utc: i64,
    pub status: RecStatus,
    pub rule_label: Option<String>,
    pub size_bytes: Option<i64>,
    pub error: Option<String>,
    /// Path or URL of the media (HLS while recording, file when complete).
    pub file_url: Option<String>,
    /// The recurring rule that created it (`custom_properties.rule.id`), when tagged.
    pub rule_ext: Option<String>,
    /// Guide id of the programme (matches series rules by title + tvg_id).
    pub tvg_id: Option<String>,
    /// What the programme said, when the recording carries it.
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    /// The guide row's id on the server.
    pub programme_ext: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Series,
    Recurring,
}

impl RuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Series => "series",
            Self::Recurring => "recurring",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleInfo {
    pub kind: RuleKind,
    /// The source's channel id when the rule names one (`channel.ext_id`).
    pub channel_ext_id: Option<String>,
    pub tvg_id: Option<String>,
    /// Recurring: the rule id; series: `"<tvg_id>|<title>"`.
    pub ext_id: String,
    pub title: String,
    pub channel_name: String,
    pub schedule: String,
}

/// What to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordRequest {
    pub channel_ext_id: String,
    pub tvg_id: Option<String>,
    pub programme_ext_id: Option<String>,
    pub title: String,
    pub subtitle: Option<String>,
    pub description: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
    pub start_utc: i64,
    pub end_utc: i64,
}
