-- URL-only artwork could have been fetched with any account sharing that URL.
-- Retire those ids so the next view registers artwork with the owning account.
DELETE FROM image_cache;
DELETE FROM image_origin;
