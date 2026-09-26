-- 020_chapter_scale_x100.sql
-- Chapter numbers move from scale *10 to *100 so two-decimal fractions
-- (0.01-0.09) no longer collide at 0: 105 (=10.5) -> 1050, 1 (=0.1) -> 10.
--
-- Applied once by sqlx::migrate!() at startup, inside sqlx's own transaction.
-- sqlx records the version in _sqlx_migrations, so re-running is a no-op.
--
-- A single `SET chapter_number = chapter_number * 10` is NOT safe against the
-- unique (provider, external_id, chapter_number) index: Postgres updates rows
-- in index order, so moving 10 -> 100 collides (23505) with the existing row
-- 100 before that row has been updated. Doing the scale-up in two steps keeps
-- every intermediate value collision-free:
--   1) x -> x*10 + 1  (cannot equal any original x*10 value: residue mod 10)
--   2) x -> x - 1     (cannot collide with any other intermediate)
-- The net effect is exactly x * 10.
--
-- media_items.chapters is a *count* (MAX/10 before, MAX/100 after) and is
-- already on the correct scale, so no data change is needed there.

UPDATE series_chapters
   SET chapter_number = chapter_number * 10 + 1;

UPDATE series_chapters
   SET chapter_number = chapter_number - 1;

UPDATE user_chapter_progress
   SET chapter_number = chapter_number * 10 + 1;

UPDATE user_chapter_progress
   SET chapter_number = chapter_number - 1;

COMMENT ON COLUMN series_chapters.chapter_number
    IS 'chapter number * 100 (100 = ch.1, 1050 = ch.10.5, 1 = ch.0.01)';

COMMENT ON COLUMN user_chapter_progress.chapter_number
    IS 'chapter number * 100 (100 = ch.1, 1050 = ch.10.5, 1 = ch.0.01)';
