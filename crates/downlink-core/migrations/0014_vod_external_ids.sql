-- External ids the source reports for a title (Dispatcharr `tmdb_id` / `imdb_id`, Xtream `tmdb`).
-- Used to find the same title on a media server (Plex).
ALTER TABLE vod_item ADD COLUMN tmdb_id TEXT;
ALTER TABLE vod_item ADD COLUMN imdb_id TEXT;
