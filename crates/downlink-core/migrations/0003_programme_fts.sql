-- Local programme search: FTS5 external-content over programme.rowid, kept in step by triggers.
CREATE VIRTUAL TABLE programme_fts USING fts5(
  title, subtitle, description,
  content='programme', content_rowid='rowid',
  tokenize='unicode61 remove_diacritics 2'
);
CREATE TRIGGER programme_fts_ai AFTER INSERT ON programme BEGIN
  INSERT INTO programme_fts(rowid, title, subtitle, description)
  VALUES (new.rowid, new.title, new.subtitle, new.description);
END;
CREATE TRIGGER programme_fts_ad AFTER DELETE ON programme BEGIN
  INSERT INTO programme_fts(programme_fts, rowid, title, subtitle, description)
  VALUES ('delete', old.rowid, old.title, old.subtitle, old.description);
END;
CREATE TRIGGER programme_fts_au AFTER UPDATE ON programme BEGIN
  INSERT INTO programme_fts(programme_fts, rowid, title, subtitle, description)
  VALUES ('delete', old.rowid, old.title, old.subtitle, old.description);
  INSERT INTO programme_fts(rowid, title, subtitle, description)
  VALUES (new.rowid, new.title, new.subtitle, new.description);
END;
