//! Media server (Plex) verbs: sign-in, server choice, and "is what's playing in my library?".
//!
//! Non-secret settings live in `app_kv` under [`PLEX_KEY`]; the account and server tokens live
//! only in the OS keychain (CLAUDE.md rule 7).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Service, parse_programme_id};
use crate::auth::SecretString;
use crate::error::{AppError, Result};
use crate::mediaserver::plex::{PLEX_TV, PlexServer, PlexTv, pick_connection};
use crate::mediaserver::{MatchKind, MediaQuery, QueryKind};
use crate::sync::now_utc;

const PLEX_KEY: &str = "mediaserver.plex";
const PLEX_ACCOUNT_REF: &str = "mediaserver:plex:account";
const PLEX_SERVER_REF: &str = "mediaserver:plex:server";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct PlexConfig {
    client_id: String,
    signed_in: bool,
    account_name: Option<String>,
    server_id: Option<String>,
    server_name: Option<String>,
    server_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexStatusDto {
    pub signed_in: bool,
    pub account_name: Option<String>,
    pub server_id: Option<String>,
    pub server_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaServerStatusDto {
    pub plex: PlexStatusDto,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexPinDto {
    pub id: i64,
    pub code: String,
    /// Page the user opens to approve the sign-in (the desktop opens it in the browser).
    pub auth_url: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlexServerDto {
    pub id: String,
    pub name: String,
    pub owned: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMatchDto {
    /// `plex` (later `jellyfin`).
    pub server: &'static str,
    pub server_name: String,
    pub kind: MatchKind,
    pub title: String,
    pub url: String,
}

/// What the player is showing.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum MediaTargetArg {
    Vod {
        id: i64,
    },
    /// `ProgrammeDto::id` (live or catch-up).
    Programme {
        id: String,
    },
    Recording {
        id: i64,
    },
}

impl Service {
    async fn plex_config(&self) -> Result<PlexConfig> {
        let mut cfg: PlexConfig = match self.store.kv_get(PLEX_KEY).await? {
            Some((v, _)) => serde_json::from_str(&v).unwrap_or_default(),
            None => PlexConfig::default(),
        };
        if cfg.client_id.is_empty() {
            cfg.client_id = uuid::Uuid::new_v4().to_string();
            self.save_plex_config(&cfg).await?;
        }
        Ok(cfg)
    }

    async fn save_plex_config(&self, cfg: &PlexConfig) -> Result<()> {
        self.store
            .kv_set(PLEX_KEY, &serde_json::to_string(cfg)?, now_utc())
            .await
    }

    async fn plex_tv(&self) -> Result<(PlexTv, PlexConfig)> {
        let cfg = self.plex_config().await?;
        let url = self
            .plex_tv_url
            .read()
            .map(|u| u.clone())
            .unwrap_or_else(|_| PLEX_TV.to_owned());
        Ok((PlexTv::new(&url, &cfg.client_id)?, cfg))
    }

    /// Point plex.tv calls elsewhere (tests use a mock server).
    #[doc(hidden)]
    pub fn set_plex_tv_url(&self, url: &str) {
        if let Ok(mut u) = self.plex_tv_url.write() {
            url.clone_into(&mut u);
        }
    }

    fn plex_account_token(&self) -> Result<SecretString> {
        self.secrets
            .get(PLEX_ACCOUNT_REF)?
            .ok_or_else(|| AppError::auth("Sign in to Plex first."))
    }

    async fn plex_server(&self) -> Result<Option<Arc<PlexServer>>> {
        let mut g = self.plex.lock().await;
        if let Some(p) = g.as_ref() {
            return Ok(Some(p.clone()));
        }
        let cfg = self.plex_config().await?;
        let (Some(url), Some(id)) = (cfg.server_url, cfg.server_id) else {
            return Ok(None);
        };
        let Some(token) = self.secrets.get(PLEX_SERVER_REF)? else {
            return Ok(None);
        };
        let p = Arc::new(PlexServer::new(&url, token, &id, &cfg.client_id)?);
        *g = Some(p.clone());
        Ok(Some(p))
    }

    pub async fn media_server_status(&self) -> Result<MediaServerStatusDto> {
        let cfg = self.plex_config().await?;
        Ok(MediaServerStatusDto {
            plex: PlexStatusDto {
                signed_in: cfg.signed_in,
                account_name: cfg.account_name,
                server_id: cfg.server_id,
                server_name: cfg.server_name,
            },
        })
    }

    /// Start the plex.tv PIN sign-in. The user approves it at `auth_url`; poll
    /// [`Service::plex_sign_in_check`] until it reports success.
    pub async fn plex_sign_in_start(&self) -> Result<PlexPinDto> {
        let (tv, _) = self.plex_tv().await?;
        let pin = tv.create_pin().await?;
        Ok(PlexPinDto {
            auth_url: tv.auth_url(&pin.code),
            id: pin.id,
            code: pin.code,
        })
    }

    /// True once the user approved the PIN (the account token is then in the keychain).
    pub async fn plex_sign_in_check(&self, pin_id: i64) -> Result<bool> {
        let (tv, mut cfg) = self.plex_tv().await?;
        let Some(token) = tv.check_pin(pin_id).await? else {
            return Ok(false);
        };
        self.secrets.set(PLEX_ACCOUNT_REF, &token)?;
        cfg.account_name = tv.account_name(&token).await.ok().flatten();
        cfg.signed_in = true;
        self.save_plex_config(&cfg).await?;
        Ok(true)
    }

    /// Servers the signed-in account can use.
    pub async fn plex_servers(&self) -> Result<Vec<PlexServerDto>> {
        let (tv, _) = self.plex_tv().await?;
        let token = self.plex_account_token()?;
        let mut out: Vec<PlexServerDto> = tv
            .servers(&token)
            .await?
            .into_iter()
            .map(|s| PlexServerDto {
                id: s.id,
                name: s.name,
                owned: s.owned,
            })
            .collect();
        out.sort_by(|a, b| b.owned.cmp(&a.owned).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    /// Use this server: find an address that answers, keep its token, start indexing.
    pub async fn plex_select_server(&self, server_id: &str) -> Result<MediaServerStatusDto> {
        let (tv, mut cfg) = self.plex_tv().await?;
        let account = self.plex_account_token()?;
        let server = tv
            .servers(&account)
            .await?
            .into_iter()
            .find(|s| s.id == server_id)
            .ok_or_else(|| AppError::local("That Plex server is no longer on your account."))?;
        let token = server.access_token.clone().unwrap_or(account);
        let url = pick_connection(&server, &token).await.ok_or_else(|| {
            AppError::network(format!(
                "Downlink could not reach {} at any of its addresses.",
                server.name
            ))
        })?;
        self.secrets.set(PLEX_SERVER_REF, &token)?;
        cfg.server_id = Some(server.id.clone());
        cfg.server_name = Some(server.name.clone());
        cfg.server_url = Some(url.clone());
        self.save_plex_config(&cfg).await?;
        let p = Arc::new(PlexServer::new(&url, token, &server.id, &cfg.client_id)?);
        *self.plex.lock().await = Some(p.clone());
        tokio::spawn(async move {
            if let Err(e) = p.index().await {
                tracing::debug!(error = %e, "plex index warm-up failed");
            }
        });
        self.media_server_status().await
    }

    /// Connect to a server by address and token, without plex.tv (tests, headless use).
    pub async fn plex_connect_direct(&self, url: &str, token: SecretString) -> Result<()> {
        let mut cfg = self.plex_config().await?;
        let probe = PlexServer::new(url, token.clone(), "", &cfg.client_id)?;
        let id = probe.identity().await?;
        self.secrets.set(PLEX_SERVER_REF, &token)?;
        cfg.signed_in = true;
        cfg.server_id = Some(id.clone());
        cfg.server_name.get_or_insert_with(|| "Plex".to_owned());
        cfg.server_url = Some(url.trim_end_matches('/').to_owned());
        self.save_plex_config(&cfg).await?;
        *self.plex.lock().await = None;
        Ok(())
    }

    /// Forget the Plex account and server (tokens leave the keychain).
    pub async fn media_server_sign_out(&self) -> Result<()> {
        self.secrets.delete(PLEX_ACCOUNT_REF)?;
        self.secrets.delete(PLEX_SERVER_REF)?;
        let cfg = self.plex_config().await?;
        self.save_plex_config(&PlexConfig {
            client_id: cfg.client_id,
            ..PlexConfig::default()
        })
        .await?;
        *self.plex.lock().await = None;
        Ok(())
    }

    async fn media_query(&self, target: &MediaTargetArg) -> Result<Option<MediaQuery>> {
        Ok(match target {
            MediaTargetArg::Vod { id } => {
                let Some(row) = self.store.vod_item(*id).await? else {
                    return Ok(None);
                };
                match row.kind.as_str() {
                    "movie" => Some(MediaQuery {
                        kind: QueryKind::Movie,
                        title: row.name,
                        year: row.year,
                        tmdb_id: row.tmdb_id,
                        imdb_id: row.imdb_id,
                        ..Default::default()
                    }),
                    "episode" => {
                        let Some(parent) = row.parent_ext.as_deref() else {
                            return Ok(None);
                        };
                        let Some(series) = self
                            .store
                            .vod_by_ext(row.source_id, "series", parent)
                            .await?
                        else {
                            return Ok(None);
                        };
                        Some(MediaQuery {
                            kind: QueryKind::Episode,
                            title: series.name,
                            year: series.year,
                            tmdb_id: series.tmdb_id,
                            imdb_id: series.imdb_id,
                            season: row.season,
                            episode: row.episode,
                        })
                    }
                    _ => None,
                }
            }
            MediaTargetArg::Programme { id } => {
                let Some((channel_id, start)) = parse_programme_id(id) else {
                    return Ok(None);
                };
                let Some(p) = self.store.programme_row(channel_id, start).await? else {
                    return Ok(None);
                };
                let Some(title) = p.title.filter(|t| !t.trim().is_empty()) else {
                    return Ok(None);
                };
                let episode = p.season.is_some() && p.episode.is_some();
                Some(MediaQuery {
                    kind: if episode {
                        QueryKind::Episode
                    } else {
                        QueryKind::Unknown
                    },
                    title,
                    season: p.season,
                    episode: p.episode,
                    ..Default::default()
                })
            }
            MediaTargetArg::Recording { id } => self
                .store
                .recording(*id)
                .await?
                .and_then(|r| r.title)
                .filter(|t| !t.trim().is_empty())
                .map(|title| MediaQuery {
                    title,
                    ..Default::default()
                }),
        })
    }

    /// The playing title on the connected media server, if it is there. `None` when no server is
    /// set up or nothing matches. Never touches a stream endpoint.
    pub async fn media_server_match(
        &self,
        target: MediaTargetArg,
    ) -> Result<Option<MediaMatchDto>> {
        let Some(server) = self.plex_server().await? else {
            return Ok(None);
        };
        let Some(q) = self.media_query(&target).await? else {
            return Ok(None);
        };
        let server_name = self
            .plex_config()
            .await?
            .server_name
            .unwrap_or_else(|| "Plex".to_owned());
        Ok(server.find(&q).await?.map(|m| MediaMatchDto {
            server: "plex",
            server_name,
            kind: m.kind,
            title: m.title,
            url: m.url,
        }))
    }
}
