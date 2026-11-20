-- M3U `tvg-shift` (hours, per entry or header), kept as seconds and applied to guide times of the
-- channel at XMLTV ingest.
ALTER TABLE channel ADD COLUMN epg_shift_secs INTEGER NOT NULL DEFAULT 0;
