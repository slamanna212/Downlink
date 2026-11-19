-- Where a content-addressed image hash came from, so a `dlimg://<hash>/<size>` lookup can be
-- fetched on a cache miss. The URL carries no credentials (auth is a header added at fetch time).
CREATE TABLE image_origin (
  hash      TEXT PRIMARY KEY,
  source_id INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  url       TEXT NOT NULL
);
