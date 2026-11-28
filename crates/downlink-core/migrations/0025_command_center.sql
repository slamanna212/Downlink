-- Saved command centers (docs/COMMAND_CENTER.md §6): a named multiview wall the user built by hand.
-- The built-in sports center is not a row. `rail_json` is an opaque JSON object the UI owns (feed
-- panels live there). Tiles reference the channel row: ids survive a resync (channels upsert on
-- source + ext id), and a channel that disappears takes its tile with it.
CREATE TABLE command_center (
  id           INTEGER PRIMARY KEY,
  name         TEXT NOT NULL,
  layout_style TEXT NOT NULL DEFAULT 'grid',  -- grid | featured
  audible      INTEGER NOT NULL DEFAULT 0,    -- slot that plays sound
  rail_json    TEXT NOT NULL DEFAULT '{}',
  created_utc  INTEGER NOT NULL,
  updated_utc  INTEGER NOT NULL
);

CREATE TABLE command_center_tile (
  center_id  INTEGER NOT NULL REFERENCES command_center(id) ON DELETE CASCADE,
  slot       INTEGER NOT NULL,                -- 0 = main player .. 8
  channel_id INTEGER NOT NULL REFERENCES channel(id) ON DELETE CASCADE,
  PRIMARY KEY (center_id, slot)
);
CREATE INDEX command_center_tile_channel_idx ON command_center_tile (channel_id);
