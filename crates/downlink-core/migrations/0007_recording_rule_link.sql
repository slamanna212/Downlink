-- Link recordings to the rule that made them (recurring: tagged by the server; series: matched by
-- title + tvg_id).
ALTER TABLE recording ADD COLUMN rule_ext TEXT;
ALTER TABLE recording ADD COLUMN tvg_id   TEXT;
