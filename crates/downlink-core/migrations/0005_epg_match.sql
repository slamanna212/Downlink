-- XMLTV matching support: the playlist's tvg-name, and the user's manual remaps.
ALTER TABLE channel ADD COLUMN tvg_name TEXT;

CREATE TABLE epg_remap (
  source_id  INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  channel_ext_id TEXT NOT NULL,
  xmltv_id   TEXT NOT NULL,
  PRIMARY KEY (source_id, channel_ext_id)
);
