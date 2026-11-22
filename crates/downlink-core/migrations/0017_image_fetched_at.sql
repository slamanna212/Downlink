-- When each cached image was downloaded, so images whose URL never changes are re-checked now and
-- then (a provider may replace a logo in place). Existing rows start from their last use.
ALTER TABLE image_cache ADD COLUMN fetched_at INTEGER NOT NULL DEFAULT 0;
UPDATE image_cache SET fetched_at = last_used;
