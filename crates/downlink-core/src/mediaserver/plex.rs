//! Plex: sign-in through plex.tv (PIN flow), server discovery, and finding a title in a server's
//! libraries.
//!
//! Matching uses an in-memory index of every movie and show section (rating key, title, year and
//! the `tmdb://` / `imdb://` / `tvdb://` guids Plex reports with `includeGuids=1`). The index is
//! rebuilt after [`INDEX_TTL`]; episodes are looked up per show (`allLeaves`) at match time.
//!
//! Tokens travel only as the `X-Plex-Token` header, never in a URL, and are registered with a
//! [`RedactionSet`] so no error or log line can carry them.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use serde::Deserialize;
use serde_json::Value;

use super::title::{strip_decoration, title_key};
use super::{MatchKind, MediaMatch, MediaQuery, QueryKind};
use crate::auth::SecretString;
use crate::error::{AppError, ErrorCode, Result};
use crate::redact::RedactionSet;
use crate::source::dispatcharr::dto::flex_string;

pub const PLEX_TV: &str = "https://plex.tv";
const PRODUCT: &str = "Downlink";
/// The library index is rebuilt this long after it was built.
pub const INDEX_TTL: Duration = Duration::from_secs(30 * 60);
const PAGE: u64 = 1000;
const MAX_PAGES: u64 = 200;

fn de_opt_string<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?
        .as_ref()
        .and_then(flex_string))
}

fn de_opt_i64<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<i64>, D::Error> {
    Ok(Option::<Value>::deserialize(d)?.as_ref().and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
    }))
}

fn headers(client_id: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(ACCEPT, HeaderValue::from_static("application/json"));
    h.insert("X-Plex-Product", HeaderValue::from_static(PRODUCT));
    h.insert("X-Plex-Device-Name", HeaderValue::from_static(PRODUCT));
    if let Ok(v) = HeaderValue::from_str(env!("CARGO_PKG_VERSION")) {
        h.insert("X-Plex-Version", v);
    }
    if let Ok(v) = HeaderValue::from_str(client_id) {
        h.insert("X-Plex-Client-Identifier", v);
    }
    h
}

fn token_header(token: &SecretString) -> Result<HeaderValue> {
    let mut v = HeaderValue::from_str(token.expose())
        .map_err(|_| AppError::auth("The Plex token is not valid."))?;
    v.set_sensitive(true);
    Ok(v)
}

fn http(timeout: Duration) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(crate::user_agent())
        .build()?)
}

fn redacted(e: reqwest::Error, secrets: &RedactionSet) -> AppError {
    let base: AppError = e.into();
    AppError::with_secrets(base.code, &base.message, secrets)
}

// ---- plex.tv ----------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub id: i64,
    pub code: String,
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub uri: String,
    pub local: bool,
    pub relay: bool,
}

#[derive(Debug, Clone)]
pub struct ServerResource {
    pub name: String,
    /// The server's machine identifier.
    pub id: String,
    pub owned: bool,
    /// Token for this server (differs from the account token on shared servers).
    pub access_token: Option<SecretString>,
    pub connections: Vec<Connection>,
}

#[derive(Deserialize)]
struct PinReply {
    #[serde(default, deserialize_with = "de_opt_i64")]
    id: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_string")]
    code: Option<String>,
    #[serde(default, rename = "authToken", deserialize_with = "de_opt_string")]
    auth_token: Option<String>,
}

#[derive(Deserialize)]
struct UserReply {
    #[serde(default, deserialize_with = "de_opt_string")]
    username: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    title: Option<String>,
}

#[derive(Deserialize)]
struct ResourceReply {
    #[serde(default, deserialize_with = "de_opt_string")]
    name: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    provides: Option<String>,
    #[serde(
        default,
        rename = "clientIdentifier",
        deserialize_with = "de_opt_string"
    )]
    client_identifier: Option<String>,
    #[serde(default, rename = "accessToken", deserialize_with = "de_opt_string")]
    access_token: Option<String>,
    #[serde(default)]
    owned: Option<bool>,
    #[serde(default)]
    connections: Vec<ConnectionReply>,
}

#[derive(Deserialize)]
struct ConnectionReply {
    #[serde(default, deserialize_with = "de_opt_string")]
    uri: Option<String>,
    #[serde(default)]
    local: Option<bool>,
    #[serde(default)]
    relay: Option<bool>,
}

/// plex.tv account API: PIN sign-in and the list of servers the account can reach.
pub struct PlexTv {
    base: reqwest::Url,
    client_id: String,
    http: reqwest::Client,
}

impl PlexTv {
    pub fn new(base: &str, client_id: &str) -> Result<Self> {
        Ok(Self {
            base: reqwest::Url::parse(base)?,
            client_id: client_id.to_owned(),
            http: http(Duration::from_secs(15))?,
        })
    }

    /// The page the user opens to approve the PIN.
    pub fn auth_url(&self, code: &str) -> String {
        let mut u = reqwest::Url::parse("https://app.plex.tv/auth").unwrap_or_else(|_| {
            // Constant URL; parsing cannot fail. Fall back to the base rather than panic.
            self.base.clone()
        });
        let frag = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("clientID", &self.client_id)
            .append_pair("code", code)
            .append_pair("context[device][product]", PRODUCT)
            .finish();
        u.set_fragment(Some(&format!("?{frag}")));
        u.to_string()
    }

    pub async fn create_pin(&self) -> Result<Pin> {
        let r = self
            .http
            .post(self.base.join("api/v2/pins")?)
            .headers(headers(&self.client_id))
            .query(&[("strong", "true")])
            .send()
            .await?;
        if !r.status().is_success() {
            return Err(AppError::from_status(r.status().as_u16(), None, "Plex"));
        }
        let p: PinReply = r.json().await?;
        match (p.id, p.code) {
            (Some(id), Some(code)) => Ok(Pin { id, code }),
            _ => Err(AppError::server(
                "Plex sent a sign-in code Downlink could not read.",
            )),
        }
    }

    /// The account token once the user approved the PIN; `None` while still waiting.
    pub async fn check_pin(&self, id: i64) -> Result<Option<SecretString>> {
        let r = self
            .http
            .get(self.base.join(&format!("api/v2/pins/{id}"))?)
            .headers(headers(&self.client_id))
            .send()
            .await?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(AppError::auth("That Plex sign-in expired. Start again."));
        }
        if !r.status().is_success() {
            return Err(AppError::from_status(r.status().as_u16(), None, "Plex"));
        }
        let p: PinReply = r.json().await?;
        Ok(p.auth_token
            .filter(|t| !t.is_empty())
            .map(SecretString::new))
    }

    pub async fn account_name(&self, token: &SecretString) -> Result<Option<String>> {
        let mut secrets = RedactionSet::new();
        secrets.add(token.expose());
        let r = self
            .http
            .get(self.base.join("api/v2/user")?)
            .headers(headers(&self.client_id))
            .header("X-Plex-Token", token_header(token)?)
            .send()
            .await
            .map_err(|e| redacted(e, &secrets))?;
        if !r.status().is_success() {
            return Err(AppError::from_status(r.status().as_u16(), None, "Plex"));
        }
        let u: UserReply = r.json().await.map_err(|e| redacted(e, &secrets))?;
        Ok(u.username.or(u.title))
    }

    /// Plex Media Servers the account can use (owned and shared).
    pub async fn servers(&self, token: &SecretString) -> Result<Vec<ServerResource>> {
        let mut secrets = RedactionSet::new();
        secrets.add(token.expose());
        let r = self
            .http
            .get(self.base.join("api/v2/resources")?)
            .headers(headers(&self.client_id))
            .header("X-Plex-Token", token_header(token)?)
            .query(&[("includeHttps", "1"), ("includeRelay", "1")])
            .send()
            .await
            .map_err(|e| redacted(e, &secrets))?;
        if r.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::auth("Plex signed you out. Sign in again."));
        }
        if !r.status().is_success() {
            return Err(AppError::from_status(r.status().as_u16(), None, "Plex"));
        }
        let body = r.bytes().await.map_err(|e| redacted(e, &secrets))?;
        let rows: Vec<ResourceReply> = serde_json::from_slice(&body)
            .map_err(|_| AppError::server("Plex sent a server list Downlink could not read."))?;
        Ok(rows
            .into_iter()
            .filter(|r| {
                r.provides
                    .as_deref()
                    .is_some_and(|p| p.split(',').any(|x| x.trim() == "server"))
            })
            .filter_map(|r| {
                Some(ServerResource {
                    id: r.client_identifier?,
                    name: r.name.unwrap_or_else(|| "Plex Media Server".to_owned()),
                    owned: r.owned.unwrap_or(false),
                    access_token: r
                        .access_token
                        .filter(|t| !t.is_empty())
                        .map(SecretString::new),
                    connections: r
                        .connections
                        .into_iter()
                        .filter_map(|c| {
                            Some(Connection {
                                uri: c.uri?,
                                local: c.local.unwrap_or(false),
                                relay: c.relay.unwrap_or(false),
                            })
                        })
                        .collect(),
                })
            })
            .collect())
    }
}

/// Connections in the order worth trying: direct local, direct remote, then relay.
pub fn connection_order(conns: &[Connection]) -> Vec<&Connection> {
    let mut v: Vec<&Connection> = conns.iter().collect();
    v.sort_by_key(|c| (c.relay, !c.local));
    v
}

#[derive(Deserialize)]
struct Envelope<T> {
    #[serde(rename = "MediaContainer")]
    mc: T,
}

#[derive(Deserialize)]
struct IdentityMc {
    #[serde(
        default,
        rename = "machineIdentifier",
        deserialize_with = "de_opt_string"
    )]
    machine_identifier: Option<String>,
}

/// The first connection of `server` that answers `/identity` as that server, within a few
/// seconds each. `/identity` is a status endpoint.
pub async fn pick_connection(server: &ServerResource, token: &SecretString) -> Option<String> {
    let client = http(Duration::from_secs(4)).ok()?;
    for c in connection_order(&server.connections) {
        let Ok(base) = reqwest::Url::parse(&c.uri) else {
            continue;
        };
        let Ok(url) = base.join("identity") else {
            continue;
        };
        let Ok(tok) = token_header(token) else {
            return None;
        };
        let Ok(r) = client
            .get(url)
            .header(ACCEPT, "application/json")
            .header("X-Plex-Token", tok)
            .send()
            .await
        else {
            continue;
        };
        if !r.status().is_success() {
            continue;
        }
        let Ok(id) = r.json::<Envelope<IdentityMc>>().await else {
            continue;
        };
        if id.mc.machine_identifier.as_deref() == Some(server.id.as_str()) {
            return Some(c.uri.trim_end_matches('/').to_owned());
        }
    }
    None
}

// ---- a server -----------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct SectionsMc {
    #[serde(default, rename = "Directory")]
    dirs: Vec<SectionDir>,
}

#[derive(Deserialize)]
struct SectionDir {
    #[serde(default, deserialize_with = "de_opt_string")]
    key: Option<String>,
    #[serde(default, rename = "type", deserialize_with = "de_opt_string")]
    kind: Option<String>,
}

#[derive(Deserialize, Default)]
struct ItemsMc {
    #[serde(default, rename = "totalSize", deserialize_with = "de_opt_i64")]
    total_size: Option<i64>,
    #[serde(default, rename = "Metadata")]
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default, rename = "ratingKey", deserialize_with = "de_opt_string")]
    rating_key: Option<String>,
    #[serde(default, deserialize_with = "de_opt_string")]
    title: Option<String>,
    #[serde(default, rename = "originalTitle", deserialize_with = "de_opt_string")]
    original_title: Option<String>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    year: Option<i64>,
    #[serde(default, deserialize_with = "de_opt_i64")]
    index: Option<i64>,
    #[serde(default, rename = "parentIndex", deserialize_with = "de_opt_i64")]
    parent_index: Option<i64>,
    #[serde(default, rename = "Guid")]
    guids: Vec<GuidTag>,
}

#[derive(Deserialize)]
struct GuidTag {
    #[serde(default, deserialize_with = "de_opt_string")]
    id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Lib {
    Movie,
    Show,
}

#[derive(Debug, Clone)]
struct Entry {
    key: String,
    lib: Lib,
    title: String,
    year: Option<i64>,
}

/// Movies and shows of every library section, keyed for lookup.
#[derive(Default)]
pub struct Index {
    entries: Vec<Entry>,
    by_guid: HashMap<(Lib, String), Vec<usize>>,
    by_title: HashMap<(Lib, String), Vec<usize>>,
}

impl Index {
    fn add(&mut self, lib: Lib, item: Item) {
        let (Some(key), Some(title)) = (item.rating_key, item.title) else {
            return;
        };
        let i = self.entries.len();
        for g in item.guids.iter().filter_map(|g| g.id.as_deref()) {
            let g = g.to_ascii_lowercase();
            if g.starts_with("tmdb://") || g.starts_with("imdb://") || g.starts_with("tvdb://") {
                self.by_guid.entry((lib, g)).or_default().push(i);
            }
        }
        let mut keys = vec![title_key(&title)];
        if let Some(o) = item.original_title.as_deref() {
            keys.push(title_key(o));
        }
        keys.sort();
        keys.dedup();
        for k in keys.into_iter().filter(|k| !k.is_empty()) {
            self.by_title.entry((lib, k)).or_default().push(i);
        }
        self.entries.push(Entry {
            key,
            lib,
            title,
            year: item.year,
        });
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn by_ids(&self, lib: Lib, q: &MediaQuery) -> Option<&Entry> {
        let guids = q
            .tmdb_id
            .iter()
            .map(|t| format!("tmdb://{t}"))
            .chain(q.imdb_id.iter().map(|t| format!("imdb://{t}")));
        for g in guids {
            if let Some(&i) = self.by_guid.get(&(lib, g)).and_then(|v| v.first()) {
                return self.entries.get(i);
            }
        }
        None
    }

    /// A unique title match (the year narrows it when known).
    fn by_title(&self, lib: Lib, title: &str, year: Option<i64>) -> Option<&Entry> {
        let (clean, year_in_title) = strip_decoration(title);
        let year = year.or(year_in_title);
        let mut keys = vec![title_key(&clean), title_key(title)];
        keys.dedup();
        for k in keys.into_iter().filter(|k| !k.is_empty()) {
            let Some(found) = self.by_title.get(&(lib, k)) else {
                continue;
            };
            let mut hits: Vec<&Entry> = found.iter().filter_map(|&i| self.entries.get(i)).collect();
            if let Some(y) = year {
                let exact: Vec<&Entry> =
                    hits.iter().copied().filter(|e| e.year == Some(y)).collect();
                if !exact.is_empty() {
                    hits = exact;
                } else {
                    hits.retain(|e| e.year.is_none_or(|ey| (ey - y).abs() <= 1));
                }
            }
            if hits.len() == 1 {
                return hits.first().copied();
            }
        }
        None
    }

    fn find(&self, lib: Lib, q: &MediaQuery) -> Option<&Entry> {
        self.by_ids(lib, q)
            .or_else(|| self.by_title(lib, &q.title, q.year))
    }
}

/// One Plex Media Server.
pub struct PlexServer {
    base: reqwest::Url,
    token: SecretString,
    machine_id: String,
    client_id: String,
    http: reqwest::Client,
    secrets: RedactionSet,
    index: tokio::sync::Mutex<Option<(Instant, Arc<Index>)>>,
    /// Per show: `(season, episode) -> rating key`, so zapping between a show's episodes asks the
    /// server once per [`INDEX_TTL`].
    episodes: tokio::sync::Mutex<HashMap<String, (Instant, Arc<EpisodeKeys>)>>,
}

type EpisodeKeys = HashMap<(i64, i64), String>;

/// Shows whose episode lists are kept at once; the oldest is dropped beyond this.
const EPISODE_CACHE_SHOWS: usize = 32;

impl std::fmt::Debug for PlexServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlexServer")
            .field("machine_id", &self.machine_id)
            .finish_non_exhaustive()
    }
}

impl PlexServer {
    pub fn new(
        base_url: &str,
        token: SecretString,
        machine_id: &str,
        client_id: &str,
    ) -> Result<Self> {
        let mut base = reqwest::Url::parse(base_url.trim())?;
        if !base.path().ends_with('/') {
            let p = format!("{}/", base.path());
            base.set_path(&p);
        }
        let mut secrets = RedactionSet::new();
        secrets.add(token.expose());
        Ok(Self {
            base,
            token,
            machine_id: machine_id.to_owned(),
            client_id: client_id.to_owned(),
            http: http(Duration::from_secs(20))?,
            secrets,
            index: tokio::sync::Mutex::new(None),
            episodes: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    pub fn machine_id(&self) -> &str {
        &self.machine_id
    }

    /// Plex Web page of an item. app.plex.tv reaches the server however the account can.
    pub fn web_url(&self, rating_key: &str) -> String {
        let key: String = url::form_urlencoded::byte_serialize(
            format!("/library/metadata/{rating_key}").as_bytes(),
        )
        .collect();
        format!(
            "https://app.plex.tv/desktop/#!/server/{}/details?key={key}",
            self.machine_id
        )
    }

    async fn get<T: serde::de::DeserializeOwned + Default>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T> {
        let r = self
            .http
            .get(self.base.join(path)?)
            .headers(headers(&self.client_id))
            .header("X-Plex-Token", token_header(&self.token)?)
            .query(query)
            .send()
            .await
            .map_err(|e| redacted(e, &self.secrets))?;
        let status = r.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::auth(
                "Plex refused the sign-in. Sign in again in Settings.",
            ));
        }
        if !status.is_success() {
            return Err(AppError::from_status(status.as_u16(), None, "Plex"));
        }
        let body = r.bytes().await.map_err(|e| redacted(e, &self.secrets))?;
        let env: Envelope<T> = serde_json::from_slice(&body).map_err(|_| {
            AppError::new(
                ErrorCode::Server,
                "Plex sent a reply Downlink could not read.",
            )
        })?;
        Ok(env.mc)
    }

    /// `/identity`: the machine identifier (checks that the address and token still work).
    pub async fn identity(&self) -> Result<String> {
        #[derive(Deserialize, Default)]
        struct Id {
            #[serde(
                default,
                rename = "machineIdentifier",
                deserialize_with = "de_opt_string"
            )]
            machine_identifier: Option<String>,
        }
        let id: Id = self.get("identity", &[]).await?;
        id.machine_identifier
            .ok_or_else(|| AppError::server("Plex did not say which server it is."))
    }

    async fn build_index(&self) -> Result<Index> {
        let sections: SectionsMc = self.get("library/sections", &[]).await?;
        let mut idx = Index::default();
        for d in sections.dirs {
            let lib = match d.kind.as_deref() {
                Some("movie") => Lib::Movie,
                Some("show") => Lib::Show,
                _ => continue,
            };
            let Some(key) = d.key else { continue };
            let mut start = 0u64;
            for _ in 0..MAX_PAGES {
                let page: ItemsMc = self
                    .get(
                        &format!("library/sections/{key}/all"),
                        &[
                            ("includeGuids", "1".to_owned()),
                            ("X-Plex-Container-Start", start.to_string()),
                            ("X-Plex-Container-Size", PAGE.to_string()),
                        ],
                    )
                    .await?;
                let n = page.items.len() as u64;
                for it in page.items {
                    idx.add(lib, it);
                }
                start += n;
                let total = page
                    .total_size
                    .and_then(|t| u64::try_from(t).ok())
                    .unwrap_or(start);
                if n == 0 || start >= total {
                    break;
                }
            }
        }
        tracing::debug!(items = idx.len(), "plex library index built");
        Ok(idx)
    }

    /// The library index, rebuilt when older than [`INDEX_TTL`]. A failed rebuild keeps serving
    /// the previous index (and retries on a later call).
    pub async fn index(&self) -> Result<Arc<Index>> {
        let mut g = self.index.lock().await;
        if let Some((at, idx)) = g.as_ref()
            && at.elapsed() < INDEX_TTL
        {
            return Ok(idx.clone());
        }
        match self.build_index().await {
            Ok(idx) => {
                let idx = Arc::new(idx);
                *g = Some((Instant::now(), idx.clone()));
                Ok(idx)
            }
            Err(e) => match g.as_mut() {
                Some((at, idx)) => {
                    tracing::debug!(error = %e, "plex index rebuild failed; keeping the old one");
                    // Retry in a few minutes rather than on every lookup.
                    *at = Instant::now()
                        .checked_sub(INDEX_TTL.saturating_sub(Duration::from_secs(300)))
                        .unwrap_or_else(Instant::now);
                    Ok(idx.clone())
                }
                None => Err(e),
            },
        }
    }

    /// Forget the index so the next lookup rebuilds it.
    pub async fn invalidate(&self) {
        *self.index.lock().await = None;
        self.episodes.lock().await.clear();
    }

    async fn find_episode(
        &self,
        show_key: &str,
        season: i64,
        episode: i64,
    ) -> Result<Option<String>> {
        if let Some((at, keys)) = self.episodes.lock().await.get(show_key)
            && at.elapsed() < INDEX_TTL
        {
            return Ok(keys.get(&(season, episode)).cloned());
        }
        let leaves: ItemsMc = self
            .get(&format!("library/metadata/{show_key}/allLeaves"), &[])
            .await?;
        let mut keys = EpisodeKeys::new();
        for e in leaves.items {
            if let (Some(s), Some(n), Some(k)) = (e.parent_index, e.index, e.rating_key) {
                keys.entry((s, n)).or_insert(k); // first wins, as a linear search would
            }
        }
        let found = keys.get(&(season, episode)).cloned();
        let mut g = self.episodes.lock().await;
        if g.len() >= EPISODE_CACHE_SHOWS
            && !g.contains_key(show_key)
            && let Some(oldest) = g
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(k, _)| k.clone())
        {
            g.remove(&oldest);
        }
        g.insert(show_key.to_owned(), (Instant::now(), Arc::new(keys)));
        Ok(found)
    }

    /// Find what `q` describes. Movies and episodes with source ids match on guid first; a title
    /// match must be unique. An episode the server lacks falls back to its show.
    pub async fn find(&self, q: &MediaQuery) -> Result<Option<MediaMatch>> {
        let idx = self.index().await?;
        let movie = |idx: &Index| {
            idx.find(Lib::Movie, q).map(|e| MediaMatch {
                kind: MatchKind::Movie,
                title: e.title.clone(),
                url: self.web_url(&e.key),
            })
        };
        let show = idx.find(Lib::Show, q).cloned();
        let found = match q.kind {
            QueryKind::Movie => movie(&idx),
            QueryKind::Episode => match show {
                Some(s) => Some(self.show_or_episode(&s, q).await?),
                None => None,
            },
            QueryKind::Unknown => match movie(&idx) {
                Some(m) => Some(m),
                None => match show {
                    Some(s) => Some(self.show_or_episode(&s, q).await?),
                    None => None,
                },
            },
        };
        Ok(found)
    }

    async fn show_or_episode(&self, show: &Entry, q: &MediaQuery) -> Result<MediaMatch> {
        debug_assert_eq!(show.lib, Lib::Show);
        if let (Some(s), Some(e)) = (q.season, q.episode)
            && let Some(key) = self.find_episode(&show.key, s, e).await?
        {
            return Ok(MediaMatch {
                kind: MatchKind::Episode,
                title: show.title.clone(),
                url: self.web_url(&key),
            });
        }
        Ok(MediaMatch {
            kind: MatchKind::Show,
            title: show.title.clone(),
            url: self.web_url(&show.key),
        })
    }
}
