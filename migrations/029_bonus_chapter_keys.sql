-- 029_bonus_chapter_keys.sql
-- Give every bonus row a distinct, stable chapter_number (its per-user progress
-- key) plus a human label for display.
--
-- Before this migration every bonus row shared chapter_number = 0 ('00'
-- prologue, non-numeric special, duplicate number) because the partial unique
-- index `series_chapters_main_unique` covers main rows only. The progress key is
-- (user_id, provider, external_id, chapter_number), so two bonuses were
-- indistinguishable - and the read bulk-fill
--   INSERT ... SELECT ... ON CONFLICT DO UPDATE
-- crashed with "ON CONFLICT DO UPDATE command cannot affect row a second time"
-- the moment a title had more than one bonus row (prod: 17 bonuses, all 0). The
-- route then returned an empty body and the clicked chapter disappeared.
--
-- Convention after this migration:
--   main  rows keep their positive x100 number (>= 0);
--   bonus rows get a distinct NEGATIVE key -(total - i) * 100 in source order,
--   so `ORDER BY chapter_number ASC` lists them oldest-first, exactly like the
--   main list. The key itself is never shown: the display label lives in
--   `chapter_label` - the original source number ('49.5') when it has one, or
--   the running counter for unnumbered specials.
--
-- Idempotent: the UPDATE only rewrites a row whose number differs from target.
ALTER TABLE series_chapters
    ADD COLUMN IF NOT EXISTS chapter_label TEXT;

COMMENT ON COLUMN series_chapters.chapter_label IS
    'display label for bonus rows (original number or counter); NULL for main rows';

UPDATE series_chapters sc
   SET chapter_number = -(n.cnt - n.rn + 1) * 100
  FROM (
      SELECT id,
             ROW_NUMBER() OVER (PARTITION BY provider, external_id
                                ORDER BY release_date ASC NULLS LAST, id ASC) AS rn,
             COUNT(*)     OVER (PARTITION BY provider, external_id) AS cnt
        FROM series_chapters
       WHERE is_bonus
  ) n
 WHERE sc.id = n.id
   AND sc.chapter_number <> -(n.cnt - n.rn + 1) * 100;
