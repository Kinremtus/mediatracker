-- Merge watching/reading into in_progress, update completed label
UPDATE tracking_entries SET status = 'in_progress' WHERE status IN ('watching', 'reading');
ALTER TABLE tracking_entries DROP CONSTRAINT IF EXISTS tracking_entries_status_check;
ALTER TABLE tracking_entries ADD CONSTRAINT tracking_entries_status_check CHECK (status IN ('planned', 'in_progress', 'completed', 'paused', 'dropped'));
