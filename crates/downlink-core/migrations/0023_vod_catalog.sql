-- Sources that deliver their whole VOD catalogue in one reply (Xtream) keep it as `vod_item` rows
-- and page, filter and search it locally, instead of holding the list in memory and in one large
-- `meta_cache` entry. `position` is the provider's order (NULL for paged sources such as
-- Dispatcharr, whose rows are a cache of what was browsed).
ALTER TABLE vod_item ADD COLUMN position INTEGER;
CREATE INDEX vod_item_browse_idx ON vod_item (source_id, kind, position);
CREATE INDEX vod_item_category_idx ON vod_item (source_id, kind, category_ext, position);

-- Title search for movies and series (local catalogues, the cached rows of paged sources and the
-- global search).
CREATE VIRTUAL TABLE vod_fts USING fts5(
  name,
  content='vod_item', content_rowid='id',
  tokenize='unicode61 remove_diacritics 2'
);
INSERT INTO vod_fts(rowid, name) SELECT id, name FROM vod_item WHERE kind IN ('movie', 'series');

CREATE TRIGGER vod_fts_ai AFTER INSERT ON vod_item WHEN new.kind IN ('movie', 'series') BEGIN
  INSERT INTO vod_fts(rowid, name) VALUES (new.id, new.name);
END;
CREATE TRIGGER vod_fts_ad AFTER DELETE ON vod_item WHEN old.kind IN ('movie', 'series') BEGIN
  INSERT INTO vod_fts(vod_fts, rowid, name) VALUES ('delete', old.id, old.name);
END;
CREATE TRIGGER vod_fts_au AFTER UPDATE ON vod_item
WHEN old.kind IN ('movie', 'series') AND old.name IS NOT new.name
BEGIN
  INSERT INTO vod_fts(vod_fts, rowid, name) VALUES ('delete', old.id, old.name);
  INSERT INTO vod_fts(rowid, name) VALUES (new.id, new.name);
END;

-- The old whole-list copies (`xtvod:<source>:movie|series`) are replaced by the rows above.
DELETE FROM meta_cache WHERE key LIKE 'xtvod:%';
