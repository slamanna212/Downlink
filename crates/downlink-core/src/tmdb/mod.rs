//! TMDB (themoviedb.org): fills the gaps the source leaves (overviews, episode titles, artwork,
//! cast with photos, certification). The user brings their own key; it lives only in the
//! keychain and is registered with a [`RedactionSet`] so no error or log line can carry it.
//!
//! Two key kinds are accepted: a v4 "API read access token" (a JWT, sent as `Authorization:
//! Bearer`) or a v3 "API key" (sent as the `api_key` query parameter, which `redact/` masks).
//!
//! Replies are normalised into [`TmdbTitle`] / [`TmdbEpisode`] here; the service caches those, not
//! the raw JSON. Artwork comes back as absolute `image.tmdb.org` URLs (public, no key needed).

use std::time::Duration;

use reqwest::header::{AUTHORIZATION, HeaderValue, RETRY_AFTER};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::SecretString;
use crate::error::{AppError, ErrorCode, Result};
use crate::mediaserver::title::{strip_decoration, title_key};
use crate::redact::RedactionSet;
use crate::source::Credit;
use crate::source::dispatcharr::dto::{flex_f64, flex_i64, flex_string};

pub const TMDB_API: &str = "https://api.themoviedb.org/3/";
/// The only host TMDB artwork is fetched from.
pub const IMAGE_HOST: &str = "image.tmdb.org";
const IMAGE_BASE: &str = "https://image.tmdb.org/t/p/";
/// Cast entries kept per title / episode.
const MAX_CAST: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TmdbKind {
    Movie,
    Tv,
}

impl TmdbKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Movie => "movie",
            Self::Tv => "tv",
        }
    }
}

/// A movie or a TV show, normalised.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TmdbTitle {
    pub kind: Option<TmdbKind>,
    pub id: i64,
    pub title: String,
    pub original_title: Option<String>,
    pub overview: Option<String>,
    pub tagline: Option<String>,
    /// Release date (movie) or first air date (show), `YYYY-MM-DD`.
    pub release_date: Option<String>,
    pub year: Option<i64>,
    pub genres: Vec<String>,
    /// Movie runtime, or a show's typical episode runtime.
    pub runtime_mins: Option<i64>,
    pub certification: Option<String>,
    pub countries: Vec<String>,
    pub networks: Vec<String>,
    /// `Returning Series`, `Ended`, `Released`…
    pub status: Option<String>,
    /// 0..10.
    pub score: Option<f64>,
    pub poster: Option<String>,
    pub backdrop: Option<String>,
    /// Transparent title logo (PNG only).
    pub logo: Option<String>,
    pub cast: Vec<Credit>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
    /// Show creators.
    pub creators: Vec<String>,
    pub imdb_id: Option<String>,
    /// YouTube video id of the trailer.
    pub trailer: Option<String>,
    pub season_count: Option<i64>,
}

/// One episode, normalised.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TmdbEpisode {
    pub season: i64,
    pub episode: i64,
    pub name: Option<String>,
    pub overview: Option<String>,
    pub air_date: Option<String>,
    pub runtime_mins: Option<i64>,
    pub still: Option<String>,
    pub score: Option<f64>,
    pub guest_stars: Vec<Credit>,
    pub directors: Vec<String>,
    pub writers: Vec<String>,
}

/// One search result.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchHit {
    pub id: i64,
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i64>,
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(flex_string)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn image(path: Option<String>, size: &str) -> Option<String> {
    let p = path?;
    p.starts_with('/').then(|| format!("{IMAGE_BASE}{size}{p}"))
}

fn names(v: Option<&Value>, key: &str) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| text(x, key)).collect(),
        _ => Vec::new(),
    }
}

fn year(date: Option<&str>) -> Option<i64> {
    date.and_then(crate::source::meta::year_of)
}

fn score(v: &Value) -> Option<f64> {
    // A handful of votes is noise; TMDB shows 0 for unrated titles.
    let votes = v.get("vote_count").and_then(flex_i64).unwrap_or(0);
    let s = v.get("vote_average").and_then(flex_f64)?;
    (votes >= 5 && s > 0.0 && s <= 10.0).then(|| (s * 10.0).round() / 10.0)
}

fn credits_cast(v: Option<&Value>) -> Vec<Credit> {
    let Some(Value::Array(a)) = v else {
        return Vec::new();
    };
    a.iter()
        .filter_map(|p| {
            Some(Credit {
                name: text(p, "name")?,
                role: text(p, "character").or_else(|| {
                    // `aggregate_credits` puts the character under roles[].
                    p.get("roles")
                        .and_then(Value::as_array)
                        .and_then(|r| r.first())
                        .and_then(|r| text(r, "character"))
                }),
                photo: image(text(p, "profile_path"), "w185"),
                guest: false,
            })
        })
        .take(MAX_CAST)
        .collect()
}

fn crew(v: Option<&Value>, jobs: &[&str]) -> Vec<String> {
    let Some(Value::Array(a)) = v else {
        return Vec::new();
    };
    let mut out: Vec<String> = a
        .iter()
        .filter(|p| {
            text(p, "job").is_some_and(|j| jobs.contains(&j.as_str()))
                || p.get("jobs").and_then(Value::as_array).is_some_and(|js| {
                    js.iter()
                        .any(|j| text(j, "job").is_some_and(|j| jobs.contains(&j.as_str())))
                })
        })
        .filter_map(|p| text(p, "name"))
        .collect();
    out.dedup();
    out.truncate(6);
    out
}

fn trailer(v: &Value) -> Option<String> {
    let list = v.get("videos")?.get("results")?.as_array()?;
    let yt = |x: &&Value| text(x, "site").as_deref() == Some("YouTube");
    let kind = |x: &&Value, k: &str| text(x, "type").as_deref() == Some(k);
    list.iter()
        .filter(yt)
        .find(|x| kind(x, "Trailer") && x.get("official").and_then(Value::as_bool) == Some(true))
        .or_else(|| list.iter().filter(yt).find(|x| kind(x, "Trailer")))
        .or_else(|| list.iter().filter(yt).find(|x| kind(x, "Teaser")))
        .and_then(|x| text(x, "key"))
}

fn logo(v: &Value, lang: &str) -> Option<String> {
    let list = v.get("images")?.get("logos")?.as_array()?;
    let png = |x: &&Value| text(x, "file_path").is_some_and(|p| p.ends_with(".png"));
    list.iter()
        .filter(png)
        .find(|x| text(x, "iso_639_1").as_deref() == Some(lang))
        .or_else(|| list.iter().find(png))
        .and_then(|x| image(text(x, "file_path"), "w500"))
}

/// Parse `movie/{id}` with `append_to_response=credits,release_dates,images,videos`.
pub fn parse_movie(v: &Value, region: &str, lang: &str) -> Option<TmdbTitle> {
    let id = v.get("id").and_then(flex_i64)?;
    let release_date = text(v, "release_date");
    let certification = v
        .get("release_dates")
        .and_then(|r| r.get("results"))
        .and_then(Value::as_array)
        .and_then(|countries| {
            let pick = |c: &str| {
                countries
                    .iter()
                    .find(|x| text(x, "iso_3166_1").as_deref() == Some(c))
                    .and_then(|x| x.get("release_dates")?.as_array().cloned())
                    .and_then(|ds| ds.iter().find_map(|d| text(d, "certification")))
            };
            pick(region).or_else(|| pick("US"))
        });
    let credits = v.get("credits");
    Some(TmdbTitle {
        kind: Some(TmdbKind::Movie),
        id,
        title: text(v, "title").unwrap_or_default(),
        original_title: text(v, "original_title").filter(|o| Some(o) != text(v, "title").as_ref()),
        overview: text(v, "overview"),
        tagline: text(v, "tagline"),
        year: year(release_date.as_deref()),
        release_date,
        genres: names(v.get("genres"), "name"),
        runtime_mins: v.get("runtime").and_then(flex_i64).filter(|r| *r > 0),
        certification,
        countries: names(v.get("production_countries"), "iso_3166_1"),
        networks: Vec::new(),
        status: text(v, "status"),
        score: score(v),
        poster: image(text(v, "poster_path"), "w500"),
        backdrop: image(text(v, "backdrop_path"), "w1280"),
        logo: logo(v, lang),
        cast: credits_cast(credits.and_then(|c| c.get("cast"))),
        directors: crew(credits.and_then(|c| c.get("crew")), &["Director"]),
        writers: crew(
            credits.and_then(|c| c.get("crew")),
            &["Screenplay", "Writer"],
        ),
        creators: Vec::new(),
        imdb_id: crate::source::vod::clean_imdb(text(v, "imdb_id")),
        trailer: trailer(v),
        season_count: None,
    })
}

/// Parse `tv/{id}` with `append_to_response=aggregate_credits,content_ratings,images,videos,external_ids`.
pub fn parse_tv(v: &Value, region: &str, lang: &str) -> Option<TmdbTitle> {
    let id = v.get("id").and_then(flex_i64)?;
    let release_date = text(v, "first_air_date");
    let certification = v
        .get("content_ratings")
        .and_then(|r| r.get("results"))
        .and_then(Value::as_array)
        .and_then(|list| {
            let pick = |c: &str| {
                list.iter()
                    .find(|x| text(x, "iso_3166_1").as_deref() == Some(c))
                    .and_then(|x| text(x, "rating"))
            };
            pick(region).or_else(|| pick("US"))
        });
    let cast = {
        let agg = credits_cast(v.get("aggregate_credits").and_then(|c| c.get("cast")));
        if agg.is_empty() {
            credits_cast(v.get("credits").and_then(|c| c.get("cast")))
        } else {
            agg
        }
    };
    Some(TmdbTitle {
        kind: Some(TmdbKind::Tv),
        id,
        title: text(v, "name").unwrap_or_default(),
        original_title: text(v, "original_name").filter(|o| Some(o) != text(v, "name").as_ref()),
        overview: text(v, "overview"),
        tagline: text(v, "tagline"),
        year: year(release_date.as_deref()),
        release_date,
        genres: names(v.get("genres"), "name"),
        runtime_mins: v
            .get("episode_run_time")
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find_map(flex_i64))
            .filter(|r| *r > 0),
        certification,
        countries: match v.get("origin_country") {
            Some(Value::Array(a)) => a.iter().filter_map(flex_string).collect(),
            _ => Vec::new(),
        },
        networks: names(v.get("networks"), "name"),
        status: text(v, "status"),
        score: score(v),
        poster: image(text(v, "poster_path"), "w500"),
        backdrop: image(text(v, "backdrop_path"), "w1280"),
        logo: logo(v, lang),
        cast,
        directors: Vec::new(),
        writers: Vec::new(),
        creators: names(v.get("created_by"), "name"),
        imdb_id: crate::source::vod::clean_imdb(
            v.get("external_ids").and_then(|e| text(e, "imdb_id")),
        ),
        trailer: trailer(v),
        season_count: v.get("number_of_seasons").and_then(flex_i64),
    })
}

/// Parse an episode (from `tv/{id}/season/{s}/episode/{e}` or a season's `episodes[]`).
pub fn parse_episode(v: &Value) -> Option<TmdbEpisode> {
    Some(TmdbEpisode {
        season: v.get("season_number").and_then(flex_i64)?,
        episode: v.get("episode_number").and_then(flex_i64)?,
        name: text(v, "name"),
        overview: text(v, "overview"),
        air_date: text(v, "air_date"),
        runtime_mins: v.get("runtime").and_then(flex_i64).filter(|r| *r > 0),
        still: image(text(v, "still_path"), "w780"),
        score: score(v),
        guest_stars: credits_cast(v.get("guest_stars")),
        directors: crew(v.get("crew"), &["Director"]),
        writers: crew(v.get("crew"), &["Writer", "Teleplay", "Story"]),
    })
}

/// Search results of `search/movie` or `search/tv`.
pub fn parse_search(v: &Value) -> Vec<SearchHit> {
    let Some(Value::Array(list)) = v.get("results") else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|r| {
            let title = text(r, "title").or_else(|| text(r, "name"))?;
            Some(SearchHit {
                id: r.get("id").and_then(flex_i64)?,
                original_title: text(r, "original_title").or_else(|| text(r, "original_name")),
                year: year(
                    text(r, "release_date")
                        .or_else(|| text(r, "first_air_date"))
                        .as_deref(),
                ),
                title,
            })
        })
        .collect()
}

/// The hit whose title is the query's (decoration, punctuation and case ignored). A year breaks
/// ties; with `strict_year` (movies) a candidate more than a year off is refused.
pub fn best_match(
    hits: &[SearchHit],
    title: &str,
    year: Option<i64>,
    strict_year: bool,
) -> Option<i64> {
    let (clean, title_year) = strip_decoration(title);
    let key = title_key(&clean);
    if key.is_empty() {
        return None;
    }
    let year = year.or(title_year);
    let same: Vec<&SearchHit> = hits
        .iter()
        .filter(|h| {
            title_key(&h.title) == key
                || h.original_title
                    .as_deref()
                    .is_some_and(|o| title_key(o) == key)
        })
        .collect();
    let near = |h: &&&SearchHit| match (year, h.year) {
        (Some(y), Some(hy)) => (y - hy).abs() <= 1,
        _ => false,
    };
    if let Some(h) = same.iter().find(near) {
        return Some(h.id);
    }
    if strict_year && year.is_some() && same.iter().any(|h| h.year.is_some()) {
        return None;
    }
    same.first().map(|h| h.id)
}

/// `(language, region, image language)` from a tag such as `en-US`.
fn locale(tag: &str) -> (String, String, String) {
    let t = tag.trim();
    let (l, r) = t.split_once('-').unwrap_or((t, "US"));
    let l = if l.len() == 2 {
        l.to_ascii_lowercase()
    } else {
        "en".to_owned()
    };
    let r = if r.len() == 2 {
        r.to_ascii_uppercase()
    } else {
        "US".to_owned()
    };
    (format!("{l}-{r}"), r, l)
}

pub struct TmdbClient {
    http: reqwest::Client,
    base: Url,
    key: SecretString,
    bearer: bool,
    secrets: RedactionSet,
    language: String,
    region: String,
    image_lang: String,
}

impl TmdbClient {
    /// `base` is [`TMDB_API`] outside tests; `language` a tag such as `en-US`.
    pub fn new(base: &str, key: SecretString, language: &str) -> Result<Self> {
        let trimmed = key.expose().trim().to_owned();
        if trimmed.is_empty() || trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(AppError::auth("That TMDB key is not valid."));
        }
        let mut secrets = RedactionSet::new();
        secrets.add(trimmed.clone());
        let bearer = trimmed.starts_with("eyJ") && trimmed.matches('.').count() == 2;
        let (language, region, image_lang) = locale(language);
        Ok(Self {
            http: crate::http::authenticated(reqwest::Client::builder())
                .timeout(Duration::from_secs(10))
                .user_agent(crate::user_agent())
                .build()?,
            base: Url::parse(base)?,
            key: SecretString::new(trimmed),
            bearer,
            secrets,
            language,
            region,
            image_lang,
        })
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    fn err(&self, e: reqwest::Error) -> AppError {
        let base: AppError = e.into();
        AppError::with_secrets(base.code, &base.message, &self.secrets)
    }

    /// GET a path under the API base. `Ok(None)` on 404.
    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Option<Value>> {
        let url = self.base.join(path)?;
        let mut req = self
            .http
            .get(url)
            .query(&[("language", self.language.as_str())])
            .query(query);
        if self.bearer {
            let mut v = HeaderValue::from_str(&format!("Bearer {}", self.key.expose()))
                .map_err(|_| AppError::auth("That TMDB key is not valid."))?;
            v.set_sensitive(true);
            req = req.header(AUTHORIZATION, v);
        } else {
            req = req.query(&[("api_key", self.key.expose())]);
        }
        let resp = req.send().await.map_err(|e| self.err(e))?;
        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if status == StatusCode::UNAUTHORIZED {
            // Not the source's credentials: never block the app over it.
            let mut e = AppError::new(ErrorCode::Auth, "TMDB rejected the API key.");
            e.blocking = false;
            return Err(e);
        }
        if !status.is_success() {
            let retry = resp
                .headers()
                .get(RETRY_AFTER)
                .and_then(|v| v.to_str().ok()?.trim().parse().ok());
            return Err(AppError::from_status(status.as_u16(), retry, "TMDB"));
        }
        let bytes = crate::http::read_body(resp, 4 * 1024 * 1024, |e| self.err(e)).await?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    /// Check the key (`GET authentication`, which needs nothing but a valid key).
    pub async fn check(&self) -> Result<()> {
        self.get("authentication", &[]).await?;
        Ok(())
    }

    pub async fn search(
        &self,
        kind: TmdbKind,
        query: &str,
        year: Option<i64>,
    ) -> Result<Vec<SearchHit>> {
        let mut q = vec![
            ("query", query.to_owned()),
            ("include_adult", "false".to_owned()),
        ];
        if let Some(y) = year {
            q.push((
                if kind == TmdbKind::Movie {
                    "year"
                } else {
                    "first_air_date_year"
                },
                y.to_string(),
            ));
        }
        Ok(self
            .get(&format!("search/{}", kind.as_str()), &q)
            .await?
            .map(|v| parse_search(&v))
            .unwrap_or_default())
    }

    /// Resolve an IMDb id to a TMDB title.
    pub async fn find_imdb(&self, imdb: &str) -> Result<Option<(TmdbKind, i64)>> {
        let Some(v) = self
            .get(
                &format!("find/{imdb}"),
                &[("external_source", "imdb_id".to_owned())],
            )
            .await?
        else {
            return Ok(None);
        };
        let first = |k: &str| {
            v.get(k)
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|x| x.get("id"))
                .and_then(flex_i64)
        };
        Ok(first("movie_results")
            .map(|id| (TmdbKind::Movie, id))
            .or_else(|| first("tv_results").map(|id| (TmdbKind::Tv, id))))
    }

    pub async fn title(&self, kind: TmdbKind, id: i64) -> Result<Option<TmdbTitle>> {
        let images = format!("{},null", self.image_lang);
        Ok(match kind {
            TmdbKind::Movie => self
                .get(
                    &format!("movie/{id}"),
                    &[
                        (
                            "append_to_response",
                            "credits,release_dates,images,videos".to_owned(),
                        ),
                        ("include_image_language", images),
                    ],
                )
                .await?
                .and_then(|v| parse_movie(&v, &self.region, &self.image_lang)),
            TmdbKind::Tv => self
                .get(
                    &format!("tv/{id}"),
                    &[
                        (
                            "append_to_response",
                            "aggregate_credits,content_ratings,images,videos,external_ids"
                                .to_owned(),
                        ),
                        ("include_image_language", images),
                    ],
                )
                .await?
                .and_then(|v| parse_tv(&v, &self.region, &self.image_lang)),
        })
    }

    pub async fn episode(&self, tv: i64, season: i64, episode: i64) -> Result<Option<TmdbEpisode>> {
        Ok(self
            .get(&format!("tv/{tv}/season/{season}/episode/{episode}"), &[])
            .await?
            .and_then(|v| parse_episode(&v)))
    }

    pub async fn season(&self, tv: i64, season: i64) -> Result<Vec<TmdbEpisode>> {
        Ok(self
            .get(&format!("tv/{tv}/season/{season}"), &[])
            .await?
            .and_then(|v| v.get("episodes").and_then(Value::as_array).cloned())
            .map(|eps| eps.iter().filter_map(parse_episode).collect())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests;
