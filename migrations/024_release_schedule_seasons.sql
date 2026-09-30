-- Phase 2: add a season dimension so TMDB series episodes can be stored and
-- shown as "S2 E5", while every existing Shikimori row stays valid at season 0
-- (rendered as plain "E12").
--
-- Backward compatible: ADD COLUMN with DEFAULT 0 backfills the 133 existing
-- rows in place. The widened UNIQUE is satisfiable because all existing rows
-- sit at season_number = 0, i.e. the new key is strictly more specific than
-- the old (provider, external_id, episode_number).
ALTER TABLE release_schedule
    ADD COLUMN season_number INT NOT NULL DEFAULT 0;

-- Auto-generated name of the inline UNIQUE in migrations/005_release_schedule.sql.
ALTER TABLE release_schedule
    DROP CONSTRAINT IF EXISTS release_schedule_provider_external_id_episode_number_key;

ALTER TABLE release_schedule
    ADD CONSTRAINT release_schedule_provider_external_id_season_episode_key
    UNIQUE (provider, external_id, season_number, episode_number);
