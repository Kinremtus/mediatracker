-- 025_chapter_count_source.sql
-- Manual chapter-count override + provenance for media_items.chapters.
-- RUNNING THE NEW COLUMNS
--   chapters_manual : user pinned the count (no auto writer may change it)
--   chapters_source : provenance string, e.g. 'manual', 'auto:kakao'
-- ADD COLUMN with a constant DEFAULT is metadata-only in PostgreSQL (no table
-- rewrite), so this is instant even on a large media_items table.

ALTER TABLE media_items
    ADD COLUMN chapters_manual BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE media_items
    ADD COLUMN chapters_source TEXT;
