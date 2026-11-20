CREATE TABLE image_cache_size (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    bytes INTEGER NOT NULL
);
INSERT INTO image_cache_size SELECT 1, coalesce(sum(bytes), 0) FROM image_cache;

CREATE TRIGGER image_cache_size_ai AFTER INSERT ON image_cache BEGIN
    UPDATE image_cache_size SET bytes = bytes + new.bytes WHERE id = 1;
END;
CREATE TRIGGER image_cache_size_au AFTER UPDATE OF bytes ON image_cache
WHEN new.bytes != old.bytes BEGIN
    UPDATE image_cache_size SET bytes = bytes + new.bytes - old.bytes WHERE id = 1;
END;
CREATE TRIGGER image_cache_size_ad AFTER DELETE ON image_cache BEGIN
    UPDATE image_cache_size SET bytes = bytes - old.bytes WHERE id = 1;
END;

CREATE INDEX image_cache_lru_idx ON image_cache (last_used, key);
