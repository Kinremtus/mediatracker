-- 028_chapter_ownership.sql
-- Clean chapter ownership: rows of series_chapters live under the key of THEIR
-- own source, not under the MangaUpdates identity key (design
-- thoughts/shared/designs/2026-10-03-chapter-ownership-design.md, :65-71).
--
--   is_bonus : bonus rows ('00' prologue, non-numeric special, duplicate number)
--              are excluded from the uniqueness rule and, later, from the
--              read-progress ratio (mc32437: 297 episodes / 281 main numbers).
--   origin   : provenance of the row - 'mu' (MangaUpdates skeleton),
--              'official_<source>' or 'manual'. Only origin='mu' rows are
--              cleanup candidates for orphan removal.
--
-- The full UNIQUE (provider, external_id, chapter_number) from 012 is replaced
-- by a PARTIAL unique index over main rows only (WHERE NOT is_bonus), otherwise
-- a bonus duplicating a main number could not be stored at all. Writers must
-- therefore say ON CONFLICT (provider, external_id, chapter_number)
-- WHERE NOT is_bonus to infer that index.
--
-- Re-key: rows under a MangaUpdates key that has a binding in media_source_ids
-- move to (source, source_id) of the winning binding. Only provider/external_id
-- are rewritten, so read/read_at/fetched_at travel with the row. Progress rows
-- in user_chapter_progress must move with the same predicate, otherwise the
-- read ratio JOINs to nothing and visually zeroes out.
--
-- Winning binding = lowest chain position for the media_type of the title
-- (official_source_for_media_type), else the global fallback rank. Sources
-- without a real fetch_source arm are filtered out (kuaikan / kakao / mangaplus
-- return an empty OfficialMeta) - see SOURCES_WITH_META in
-- src/services/external/official_meta.rs (arms of fetch_source, :55-77, batch 2).
-- The list below must stay identical to that constant (10 sources) - the unit
-- test in batch 2 guards it.
--
-- Fully idempotent: IF [NOT] EXISTS everywhere plus a key-guarded UPDATE, so
-- re-running the statements by hand is a no-op (a row already sitting under its
-- source key is not matched by the MangaUpdates guard). No conflict handling on
-- purpose: before this migration every writer used MangaUpdates keys only, so a
-- re-keyed row cannot collide with an existing official-key row, and an
-- unexpected collision aborts app startup instead of silently losing data.
--
-- Statement order is ALTER -> index -> UPDATE, so both UPDATEs see is_bonus.

ALTER TABLE series_chapters
    ADD COLUMN IF NOT EXISTS is_bonus BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE series_chapters
    ADD COLUMN IF NOT EXISTS origin TEXT NOT NULL DEFAULT 'mu';

COMMENT ON COLUMN series_chapters.is_bonus IS
    'bonus episode (prologue, special, duplicate number) - excluded from the unique index and from the read ratio';

COMMENT ON COLUMN series_chapters.origin IS
    'row provenance: mu | official_<source> | manual';

-- The inline UNIQUE of 012 got PostgreSQL default name. Dropping the constraint
-- frees the bonus-duplicate slot, the partial index keeps uniqueness for main rows.
ALTER TABLE series_chapters
    DROP CONSTRAINT IF EXISTS series_chapters_provider_external_id_chapter_number_key;

CREATE UNIQUE INDEX IF NOT EXISTS series_chapters_main_unique
    ON series_chapters (provider, external_id, chapter_number)
    WHERE NOT is_bonus;

-- Winner per MangaUpdates title: chain position first, global rank as fallback
-- (a title can have no media_items row yet - drawer preview before adding).
WITH supports_meta (source, global_rank) AS (
    VALUES ('bilibili', 1), ('naver', 2), ('daum', 3), ('ridibooks', 4),
           ('comicwalker', 5), ('mangaup', 6), ('alphapolis', 7),
           ('syosetu', 8), ('kakuyomu', 9), ('qidian', 10)
),
chain (media_type, source, chain_rank) AS (
    VALUES ('manhua', 'kuaikan', 1), ('manhua', 'bilibili', 2),
           ('manhwa', 'kakao', 1), ('manhwa', 'naver', 2),
           ('manhwa', 'daum', 3), ('manhwa', 'ridibooks', 4),
           ('manga', 'mangaplus', 1), ('manga', 'comicwalker', 2),
           ('manga', 'mangaup', 3), ('manga', 'alphapolis', 4),
           ('novel', 'syosetu', 1), ('novel', 'kakuyomu', 2),
           ('novel', 'qidian', 3), ('novel', 'ridibooks', 4),
           ('novel', 'alphapolis', 5)
),
ranked AS (
    SELECT b.provider, b.external_id, b.source, b.source_id,
           ROW_NUMBER() OVER (
               PARTITION BY b.provider, b.external_id
               ORDER BY COALESCE(c.chain_rank, 100 + sm.global_rank),
                        sm.global_rank,
                        b.source
           ) AS rn
      FROM media_source_ids b
      JOIN supports_meta sm ON sm.source = b.source
      LEFT JOIN media_items mi
             ON mi.provider = b.provider AND mi.external_id = b.external_id
      LEFT JOIN chain c
             ON c.media_type = mi.media_type AND c.source = b.source
),
winner AS (
    SELECT provider, external_id, source, source_id
      FROM ranked
     WHERE rn = 1
)
UPDATE series_chapters sc
   SET provider = w.source,
       external_id = w.source_id
  FROM winner w
 WHERE sc.provider = w.provider
   AND sc.external_id = w.external_id
   AND sc.provider = 'mangaupdates';

WITH supports_meta (source, global_rank) AS (
    VALUES ('bilibili', 1), ('naver', 2), ('daum', 3), ('ridibooks', 4),
           ('comicwalker', 5), ('mangaup', 6), ('alphapolis', 7),
           ('syosetu', 8), ('kakuyomu', 9), ('qidian', 10)
),
chain (media_type, source, chain_rank) AS (
    VALUES ('manhua', 'kuaikan', 1), ('manhua', 'bilibili', 2),
           ('manhwa', 'kakao', 1), ('manhwa', 'naver', 2),
           ('manhwa', 'daum', 3), ('manhwa', 'ridibooks', 4),
           ('manga', 'mangaplus', 1), ('manga', 'comicwalker', 2),
           ('manga', 'mangaup', 3), ('manga', 'alphapolis', 4),
           ('novel', 'syosetu', 1), ('novel', 'kakuyomu', 2),
           ('novel', 'qidian', 3), ('novel', 'ridibooks', 4),
           ('novel', 'alphapolis', 5)
),
ranked AS (
    SELECT b.provider, b.external_id, b.source, b.source_id,
           ROW_NUMBER() OVER (
               PARTITION BY b.provider, b.external_id
               ORDER BY COALESCE(c.chain_rank, 100 + sm.global_rank),
                        sm.global_rank,
                        b.source
           ) AS rn
      FROM media_source_ids b
      JOIN supports_meta sm ON sm.source = b.source
      LEFT JOIN media_items mi
             ON mi.provider = b.provider AND mi.external_id = b.external_id
      LEFT JOIN chain c
             ON c.media_type = mi.media_type AND c.source = b.source
),
winner AS (
    SELECT provider, external_id, source, source_id
      FROM ranked
     WHERE rn = 1
)
UPDATE user_chapter_progress p
   SET provider = w.source,
       external_id = w.source_id
  FROM winner w
 WHERE p.provider = w.provider
   AND p.external_id = w.external_id
   AND p.provider = 'mangaupdates';