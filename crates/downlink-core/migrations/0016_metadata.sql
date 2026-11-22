-- Richer titles: extra fields the source reports (cast, director, release date, certification) and
-- a cache for lookups that are not catalog rows (the server's programme details, TMDB replies).

-- JSON `VodExtra` from the source's detail call; NULL until the title has been opened once.
ALTER TABLE vod_item ADD COLUMN meta TEXT;

-- What the recording's programme said (series-rule and our own recordings carry it).
ALTER TABLE recording ADD COLUMN description   TEXT;
ALTER TABLE recording ADD COLUMN season        INTEGER;
ALTER TABLE recording ADD COLUMN episode       INTEGER;
ALTER TABLE recording ADD COLUMN programme_ext TEXT;

-- Key/value cache with a fetch time; readers decide freshness. `body` is our own normalised JSON,
-- never a raw server reply, and never carries credentials.
CREATE TABLE meta_cache (
  key        TEXT PRIMARY KEY,
  body       TEXT NOT NULL,
  fetched_at INTEGER NOT NULL
);
CREATE INDEX meta_cache_age_idx ON meta_cache (fetched_at);
