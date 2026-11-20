-- Per-source opt-in: use Dispatcharr's HLS output for live channels (PLAN, docs/dispatcharr-endpoints.md "Live HLS")
ALTER TABLE source ADD COLUMN hls_transport INTEGER NOT NULL DEFAULT 0;
