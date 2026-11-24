-- Programme artwork from the guide itself (XMLTV <icon src>), so heroes and cards can show a
-- picture without a network lookup. NULL when the guide had none.
ALTER TABLE programme ADD COLUMN icon TEXT;
