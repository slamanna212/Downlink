-- Sports tracking (docs/SPORTS.md §4): followed-league matches for now .. +7 days, rebuilt after
-- every guide/catalog sync and whenever `sports.follow` changes. Pure cache: safe to empty.
CREATE TABLE sports_match (
  channel_id    INTEGER NOT NULL REFERENCES channel(id) ON DELETE CASCADE,
  start_utc     INTEGER,                   -- programme start; parsed best effort or NULL for channel-name matches
  end_utc       INTEGER,                   -- NULL for channel-name matches
  league        TEXT NOT NULL,             -- catalog key
  teams         TEXT NOT NULL DEFAULT '',  -- comma-separated team keys that matched (followed or not)
  session       TEXT,                      -- motorsport only: race | qualifying | practice | shows
  live          INTEGER NOT NULL,          -- 1 live (flagged or inferred), 0 replay or unknown
  live_inferred INTEGER NOT NULL DEFAULT 0, -- 1 when the source never flags LIVE ("likely live")
  score         INTEGER NOT NULL,
  source        TEXT NOT NULL,             -- programme | channel
  PRIMARY KEY (channel_id, league, start_utc)
);
CREATE INDEX sports_match_time_idx ON sports_match (start_utc);
