-- 020_chapter_scale_x100.sql
-- Chapter numbers move from scale *10 to *100 so two-decimal fractions
-- (0.01-0.09) no longer collide at 0: 105 (=10.5) -> 1050, 1 (=0.1) -> 10.
--
-- Applied once by sqlx::migrate!() at startup, inside sqlx's own transaction.
-- sqlx records the version in _sqlx_migrations, so re-running is a no-op.
-- Uniqueness is preserved because every row is multiplied by the same factor.
--
-- media_items.chapters is a *count* (MAX/10 before, MAX/100 after) and is
-- already on the correct scale, so no data change is needed there.

UPDATE series_chapters
   SET chapter_number = chapter_number * 10;

UPDATE user_chapter_progress
   SET chapter_number = chapter_number * 10;

COMMENT ON COLUMN series_chapters.chapter_number
    IS 'chapter number * 100 (100 = ch.1, 1050 = ch.10.5, 1 = ch.0.01)';

COMMENT ON COLUMN user_chapter_progress.chapter_number
    IS 'chapter number * 100 (100 = ch.1, 1050 = ch.10.5, 1 = ch.0.01)';
