-- Downlink schema v1 (PLAN §7.1). Forward-only. UTC integers everywhere. No secrets, ever.

CREATE TABLE source (
  id             INTEGER PRIMARY KEY,
  uuid           TEXT NOT NULL UNIQUE,
  kind           TEXT NOT NULL,
  name           TEXT NOT NULL,
  base_url       TEXT,
  secret_ref     TEXT,
  username       TEXT,
  user_level     TEXT,
  server_version TEXT,
  server_tz      TEXT,
  caps           INTEGER NOT NULL DEFAULT 0,
  tls_insecure   INTEGER NOT NULL DEFAULT 0,
  last_sync_at   INTEGER,
  created_at     INTEGER NOT NULL
);

CREATE TABLE channel_group (
  id        INTEGER PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  ext_id    TEXT NOT NULL,
  name      TEXT NOT NULL,
  UNIQUE (source_id, ext_id)
);

CREATE TABLE channel (
  id               INTEGER PRIMARY KEY,
  source_id        INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  ext_id           TEXT NOT NULL,
  uuid             TEXT,
  number           REAL,
  name             TEXT NOT NULL,
  group_id         INTEGER REFERENCES channel_group(id) ON DELETE SET NULL,
  tvg_id           TEXT,
  logo_key         TEXT,
  quality          TEXT,
  is_adult         INTEGER NOT NULL DEFAULT 0,
  is_radio         INTEGER NOT NULL DEFAULT 0,
  is_drm           INTEGER NOT NULL DEFAULT 0,
  catchup_days     INTEGER NOT NULL DEFAULT 0,
  catchup_template TEXT,
  stream_redirect  INTEGER NOT NULL DEFAULT 0,
  play_url         TEXT,
  play_headers     TEXT,
  epg_match        TEXT,
  sort_key         TEXT,
  -- text form of `number` so FTS5 can index it (external-content tables need real columns)
  number_text      TEXT GENERATED ALWAYS AS (
                     CASE WHEN number IS NULL THEN NULL
                          ELSE rtrim(rtrim(printf('%.4f', number), '0'), '.') END
                   ) VIRTUAL,
  UNIQUE (source_id, ext_id)
);
CREATE INDEX channel_group_idx  ON channel (source_id, group_id);
CREATE INDEX channel_number_idx ON channel (source_id, number);
CREATE INDEX channel_tvg_idx    ON channel (source_id, tvg_id);

CREATE TABLE channel_stream (
  channel_id INTEGER NOT NULL REFERENCES channel(id) ON DELETE CASCADE,
  stream_id  TEXT NOT NULL,
  position   INTEGER NOT NULL,
  PRIMARY KEY (channel_id, stream_id)
);

CREATE TABLE programme (
  channel_id  INTEGER NOT NULL REFERENCES channel(id) ON DELETE CASCADE,
  start_utc   INTEGER NOT NULL,
  end_utc     INTEGER NOT NULL,
  ext_id      TEXT,
  title       TEXT,
  subtitle    TEXT,
  description TEXT,
  season      INTEGER,
  episode     INTEGER,
  flags       INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (channel_id, start_utc)
);
CREATE INDEX programme_window_idx ON programme (start_utc, end_utc);

CREATE TABLE vod_item (
  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  ext_id TEXT NOT NULL, parent_id INTEGER,
  name TEXT NOT NULL, year INTEGER, rating TEXT, genre TEXT, description TEXT,
  season INTEGER, episode INTEGER, duration_secs INTEGER, art_key TEXT, updated_at INTEGER,
  UNIQUE (source_id, kind, ext_id)
);

CREATE TABLE recording (
  id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  ext_id TEXT NOT NULL, channel_id INTEGER, start_utc INTEGER, end_utc INTEGER,
  state TEXT,
  title TEXT, subtitle TEXT, rule_kind TEXT,
  UNIQUE (source_id, ext_id)
);

CREATE TABLE favourite (
  channel_id INTEGER PRIMARY KEY REFERENCES channel(id) ON DELETE CASCADE,
  position INTEGER, updated_at INTEGER NOT NULL, deleted INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE recent (
  channel_id INTEGER PRIMARY KEY REFERENCES channel(id) ON DELETE CASCADE,
  last_played_at INTEGER NOT NULL
);
CREATE TABLE resume (
  source_id INTEGER NOT NULL, kind TEXT NOT NULL,
  item_key TEXT NOT NULL, position_ms INTEGER NOT NULL, duration_ms INTEGER,
  updated_at INTEGER NOT NULL, PRIMARY KEY (source_id, kind, item_key)
);

CREATE TABLE open_session (
  source_id INTEGER NOT NULL, kind TEXT NOT NULL, session_id TEXT NOT NULL,
  PRIMARY KEY (source_id, kind, session_id)
);

CREATE TABLE image_cache (
  key TEXT PRIMARY KEY, path TEXT NOT NULL, bytes INTEGER NOT NULL, last_used INTEGER NOT NULL
);

CREATE TABLE sync_state (
  source_id INTEGER NOT NULL, entity TEXT NOT NULL, last_ok INTEGER,
  last_error TEXT, PRIMARY KEY (source_id, entity)
);

-- Search: FTS5 external-content over channel, kept in step by triggers.
CREATE VIRTUAL TABLE channel_fts USING fts5(
  name, number_text, tvg_id,
  content='channel', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER channel_fts_ai AFTER INSERT ON channel BEGIN
  INSERT INTO channel_fts(rowid, name, number_text, tvg_id)
  VALUES (new.id, new.name, new.number_text, new.tvg_id);
END;
CREATE TRIGGER channel_fts_ad AFTER DELETE ON channel BEGIN
  INSERT INTO channel_fts(channel_fts, rowid, name, number_text, tvg_id)
  VALUES ('delete', old.id, old.name, old.number_text, old.tvg_id);
END;
CREATE TRIGGER channel_fts_au AFTER UPDATE ON channel BEGIN
  INSERT INTO channel_fts(channel_fts, rowid, name, number_text, tvg_id)
  VALUES ('delete', old.id, old.name, old.number_text, old.tvg_id);
  INSERT INTO channel_fts(rowid, name, number_text, tvg_id)
  VALUES (new.id, new.name, new.number_text, new.tvg_id);
END;
