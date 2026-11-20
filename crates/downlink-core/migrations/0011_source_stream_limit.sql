-- Account stream limit learned at connect (Dispatcharr `stream_limit`, Xtream `max_connections`);
-- NULL = unknown or unlimited. Drives the multiview stream budget (PLAN §4.11).
ALTER TABLE source ADD COLUMN stream_limit INTEGER;
