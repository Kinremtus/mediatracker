-- 031_notification_log_season.sql
-- The notification dedup key needs the season: a TMDB series keeps one
-- `external_id` across seasons and episode numbers restart at 1 per season,
-- so the old key (user, provider, external_id, episode_number) treated S2E5
-- as an already-sent S1E5 and silently skipped every season after the first.
--
-- Existing rows predate the column; they belong to the only season they
-- could have been sent for, so they default to 1.

ALTER TABLE notification_log
    ADD COLUMN IF NOT EXISTS season_number INTEGER NOT NULL DEFAULT 1;

-- Rebuild the dedup index over the widened key. The old index was already
-- unique, so there are no duplicates to collapse.
DROP INDEX IF EXISTS uq_notification_log;
CREATE UNIQUE INDEX IF NOT EXISTS uq_notification_log
    ON notification_log(user_id, provider, external_id, season_number, episode_number);
