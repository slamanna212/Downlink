CREATE INDEX channel_browse_idx ON channel ((number IS NULL), number, sort_key);
CREATE INDEX channel_group_browse_idx ON channel (group_id, (number IS NULL), number, sort_key);
CREATE INDEX channel_source_browse_idx ON channel (source_id, (number IS NULL), number, sort_key);

DROP TRIGGER channel_fts_au;
CREATE TRIGGER channel_fts_au AFTER UPDATE ON channel
WHEN old.id IS NOT new.id OR old.name IS NOT new.name
  OR old.number_text IS NOT new.number_text OR old.tvg_id IS NOT new.tvg_id
BEGIN
  INSERT INTO channel_fts(channel_fts, rowid, name, number_text, tvg_id)
  VALUES ('delete', old.id, old.name, old.number_text, old.tvg_id);
  INSERT INTO channel_fts(rowid, name, number_text, tvg_id)
  VALUES (new.id, new.name, new.number_text, new.tvg_id);
END;

-- Index single-character prefixes so live search does not have to merge the postings
-- for every term beginning with that character after large guide imports.
DROP TRIGGER programme_fts_ai;
DROP TRIGGER programme_fts_ad;
DROP TRIGGER programme_fts_au;
DROP TABLE programme_fts;
CREATE VIRTUAL TABLE programme_fts USING fts5(
  title, subtitle, description,
  content='programme', content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2', prefix='1'
);
INSERT INTO programme_fts(programme_fts) VALUES ('rebuild');

CREATE TRIGGER programme_fts_ai AFTER INSERT ON programme BEGIN
  INSERT INTO programme_fts(rowid, title, subtitle, description)
  VALUES (new.rowid, new.title, new.subtitle, new.description);
END;
CREATE TRIGGER programme_fts_ad AFTER DELETE ON programme BEGIN
  INSERT INTO programme_fts(programme_fts, rowid, title, subtitle, description)
  VALUES ('delete', old.rowid, old.title, old.subtitle, old.description);
END;
CREATE TRIGGER programme_fts_au AFTER UPDATE ON programme
WHEN old.rowid IS NOT new.rowid OR old.title IS NOT new.title
  OR old.subtitle IS NOT new.subtitle OR old.description IS NOT new.description
BEGIN
  INSERT INTO programme_fts(programme_fts, rowid, title, subtitle, description)
  VALUES ('delete', old.rowid, old.title, old.subtitle, old.description);
  INSERT INTO programme_fts(rowid, title, subtitle, description)
  VALUES (new.rowid, new.title, new.subtitle, new.description);
END;
