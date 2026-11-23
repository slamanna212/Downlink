//! Media servers (Plex today; Jellyfin later): find what is playing in the user's own library so
//! the player can offer to open it there. Lookups only touch the media server, never a stream.

pub mod plex;
pub mod title;

use serde::Serialize;

/// What kind of title a query describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueryKind {
    Movie,
    Episode,
    /// Live / catch-up / recordings without season and episode: try a movie, then a show.
    #[default]
    Unknown,
}

/// What is playing, as far as matching needs.
#[derive(Debug, Clone, Default)]
pub struct MediaQuery {
    pub kind: QueryKind,
    /// Movie title, or the show's title for an episode (decoration is stripped when matching).
    pub title: String,
    pub year: Option<i64>,
    /// Ids of the movie, or of the show for an episode.
    pub tmdb_id: Option<String>,
    pub imdb_id: Option<String>,
    pub season: Option<i64>,
    pub episode: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchKind {
    Movie,
    Show,
    Episode,
}

/// A title found on the media server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaMatch {
    pub kind: MatchKind,
    /// The server's title (the show's for an episode).
    pub title: String,
    /// Web page of the item on the media server.
    pub url: String,
}
