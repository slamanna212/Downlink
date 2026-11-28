-- News feeds (docs/COMMAND_CENTER.md §6.2): user-added RSS/Atom feeds and their recent items.
-- A feed URL that carries a credential (userinfo or a token query parameter) lives in the OS
-- keychain under `secret_ref`; `url` then holds the credential-free display form (rule 7).
CREATE TABLE feed (
  id             INTEGER PRIMARY KEY,
  url            TEXT NOT NULL UNIQUE,     -- the feed URL, or its masked form when secret_ref is set
  secret_ref     TEXT,                     -- keychain reference of the full URL, never the URL
  title          TEXT,
  site_url       TEXT,
  etag           TEXT,
  last_modified  TEXT,
  last_fetch_utc INTEGER,
  last_ok_utc    INTEGER,
  last_error     TEXT,                     -- redacted, URL-free
  created_utc    INTEGER NOT NULL
);

-- Items from the last 48 h, at most 200 per feed (pruned after every fetch).
CREATE TABLE feed_item (
  feed_id       INTEGER NOT NULL REFERENCES feed(id) ON DELETE CASCADE,
  guid          TEXT NOT NULL,
  title         TEXT NOT NULL,
  link          TEXT,
  summary       TEXT,                      -- plain text, truncated
  published_utc INTEGER,
  fetched_utc   INTEGER NOT NULL,          -- first seen
  PRIMARY KEY (feed_id, guid)
);
CREATE INDEX feed_item_published_idx ON feed_item (feed_id, published_utc);
