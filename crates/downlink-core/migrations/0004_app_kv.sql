-- Small synced key/value state (e.g. the `prefs` object of the Dispatcharr app-data document).
CREATE TABLE app_kv (
  key        TEXT PRIMARY KEY,
  value      TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
