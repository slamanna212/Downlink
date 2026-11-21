-- VOD categories and how many titles each holds, recomputed only by the catalog sync so the
-- Movies/Series sidebar never has to count categories on the provider per view.
-- item_count NULL = the provider could not say (listed, not hidden).
CREATE TABLE vod_category (
  source_id  INTEGER NOT NULL REFERENCES source(id) ON DELETE CASCADE,
  kind       TEXT    NOT NULL,
  ext_id     TEXT    NOT NULL,
  name       TEXT    NOT NULL,
  item_count INTEGER,
  PRIMARY KEY (source_id, kind, ext_id)
);
