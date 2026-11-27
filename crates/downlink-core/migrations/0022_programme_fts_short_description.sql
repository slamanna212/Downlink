-- Programme search indexes the first 300 characters of each description (the opening sentences,
-- where the useful words are) instead of the whole text. With millions of guide rows the full
-- descriptions made the index larger than the guide itself. Titles and subtitles stay whole.
--
-- The index is external-content, so the triggers below must always index and delete exactly the
-- same `substr(...)` value. Never run the FTS 'rebuild' command on this table: it would read the
-- full descriptions from `programme` and the delete triggers would no longer match.
DROP TRIGGER programme_fts_ai;
DROP TRIGGER programme_fts_ad;
DROP TRIGGER programme_fts_au;
DROP TABLE programme_fts;
CREATE VIRTUAL TABLE programme_fts USING fts5(
  title, subtitle, description,
  content='programme', content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2', prefix='1'
);
INSERT INTO programme_fts(rowid, title, subtitle, description)
  SELECT rowid, title, subtitle, substr(description, 1, 300) FROM programme;

CREATE TRIGGER programme_fts_ai AFTER INSERT ON programme BEGIN
  INSERT INTO programme_fts(rowid, title, subtitle, description)
  VALUES (new.rowid, new.title, new.subtitle, substr(new.description, 1, 300));
END;
CREATE TRIGGER programme_fts_ad AFTER DELETE ON programme BEGIN
  INSERT INTO programme_fts(programme_fts, rowid, title, subtitle, description)
  VALUES ('delete', old.rowid, old.title, old.subtitle, substr(old.description, 1, 300));
END;
CREATE TRIGGER programme_fts_au AFTER UPDATE ON programme
WHEN old.rowid IS NOT new.rowid OR old.title IS NOT new.title
  OR old.subtitle IS NOT new.subtitle
  OR substr(old.description, 1, 300) IS NOT substr(new.description, 1, 300)
BEGIN
  INSERT INTO programme_fts(programme_fts, rowid, title, subtitle, description)
  VALUES ('delete', old.rowid, old.title, old.subtitle, substr(old.description, 1, 300));
  INSERT INTO programme_fts(rowid, title, subtitle, description)
  VALUES (new.rowid, new.title, new.subtitle, substr(new.description, 1, 300));
END;
