-- Associated (alternative) titles from MangaUpdates `associated` array.
-- TEXT[] follows the existing multi-value pattern on media_items
-- (authors, artists, genres, categories, ...); GIN index mirrors migration 008.
-- Existing rows keep the default '{}' and are backfilled lazily on the next
-- metadata refresh (admin "refresh details" / periodic refresh_counts).
ALTER TABLE media_items
    ADD COLUMN associated_titles TEXT[] NOT NULL DEFAULT '{}';

CREATE INDEX IF NOT EXISTS idx_media_associated_titles_gin
    ON media_items USING gin(associated_titles);
