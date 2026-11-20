-- M4: VOD browse cache keys and DVR rules. Local row ids are what the UI sees.
ALTER TABLE vod_item ADD COLUMN uuid         TEXT;
ALTER TABLE vod_item ADD COLUMN container    TEXT;
ALTER TABLE vod_item ADD COLUMN category_ext TEXT;
ALTER TABLE vod_item ADD COLUMN backdrop_key TEXT;
ALTER TABLE vod_item ADD COLUMN parent_ext   TEXT;   -- episode -> series ext id
CREATE INDEX vod_item_parent_idx ON vod_item (source_id, parent_ext);

CREATE TABLE recording_rule (
  id         INTEGER PRIMARY KEY,
  source_id  INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  kind       TEXT NOT NULL,            -- series | recurring
  ext_id     TEXT NOT NULL,            -- recurring: rule id; series: "<tvg_id>|<title>"
  title      TEXT NOT NULL,
  channel_name TEXT NOT NULL DEFAULT '',
  schedule   TEXT NOT NULL DEFAULT '',
  UNIQUE (source_id, kind, ext_id)
);

ALTER TABLE recording ADD COLUMN size_bytes INTEGER;
ALTER TABLE recording ADD COLUMN error TEXT;
ALTER TABLE recording ADD COLUMN rule_label TEXT;
ALTER TABLE recording ADD COLUMN file_url TEXT;
