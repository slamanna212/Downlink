ALTER TABLE channel_group ADD COLUMN position INTEGER NOT NULL DEFAULT 0;

-- Existing rows use their insertion order until the next source sync.
UPDATE channel_group SET position = id;
