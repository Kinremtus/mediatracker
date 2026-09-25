-- 015_user_progress.sql
-- Per-user episode/chapter progress, decoupled from the shared catalog
-- tables (anime_episodes / series_chapters / tmdb_episodes). Catalog
-- `watched` / `read` columns stay untouched: they remain the global
-- "what has been released / what the catalog knows" data, while these
-- tables hold each user's own checkmarks.
--
-- Composite PKs guarantee one row per (user, item, episode/chapter) and
-- make the eventual writes idempotent via ON CONFLICT.

-- ---------------------------------------------------------------------
-- anime episodes (provider = 'mal', external_id = mal_id::text)
-- ---------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS user_episode_progress (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider VARCHAR(20) NOT NULL,
    external_id VARCHAR(100) NOT NULL,
    episode_number INTEGER NOT NULL,
    watched BOOLEAN NOT NULL DEFAULT FALSE,
    watched_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, provider, external_id, episode_number)
);

CREATE INDEX IF NOT EXISTS idx_user_episode_progress_lookup
    ON user_episode_progress(provider, external_id, episode_number);

CREATE INDEX IF NOT EXISTS idx_user_episode_progress_watched
    ON user_episode_progress(user_id, provider, external_id)
    WHERE watched = TRUE;

CREATE INDEX IF NOT EXISTS idx_user_episode_progress_watched_at
    ON user_episode_progress(watched_at)
    WHERE watched_at IS NOT NULL;

-- ---------------------------------------------------------------------
-- manga / manhwa / novel chapters (provider = 'mangaupdates',
-- external_id = series_id; chapter_number is stored as ch * 10)
-- ---------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS user_chapter_progress (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider VARCHAR(20) NOT NULL,
    external_id VARCHAR(100) NOT NULL,
    chapter_number INTEGER NOT NULL,
    read BOOLEAN NOT NULL DEFAULT FALSE,
    read_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, provider, external_id, chapter_number)
);

CREATE INDEX IF NOT EXISTS idx_user_chapter_progress_lookup
    ON user_chapter_progress(provider, external_id, chapter_number);

CREATE INDEX IF NOT EXISTS idx_user_chapter_progress_read
    ON user_chapter_progress(user_id, provider, external_id)
    WHERE read = TRUE;

CREATE INDEX IF NOT EXISTS idx_user_chapter_progress_read_at
    ON user_chapter_progress(read_at)
    WHERE read_at IS NOT NULL;

-- ---------------------------------------------------------------------
-- TMDB episodes (movies/series/dramas/cartoons). provider is implicit
-- ('tmdb'); external_id is the TMDB id, matching tmdb_episodes.external_id.
-- ---------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS user_tmdb_episode_progress (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    external_id VARCHAR(100) NOT NULL,
    season_number INTEGER NOT NULL,
    episode_number INTEGER NOT NULL,
    watched BOOLEAN NOT NULL DEFAULT FALSE,
    watched_at TIMESTAMPTZ,
    PRIMARY KEY (user_id, external_id, season_number, episode_number)
);

CREATE INDEX IF NOT EXISTS idx_user_tmdb_episode_progress_lookup
    ON user_tmdb_episode_progress(external_id, season_number, episode_number);

CREATE INDEX IF NOT EXISTS idx_user_tmdb_episode_progress_watched
    ON user_tmdb_episode_progress(user_id, external_id, season_number)
    WHERE watched = TRUE;

CREATE INDEX IF NOT EXISTS idx_user_tmdb_episode_progress_watched_at
    ON user_tmdb_episode_progress(watched_at)
    WHERE watched_at IS NOT NULL;

-- ---------------------------------------------------------------------
-- Backfill: copy the existing global `watched` / `read` flags into
-- per-user rows, but only for users who actually track the media item.
-- Idempotent / repeat-safe: DISTINCT ON collapses the OR-join (which can
-- match several media_items for the same MAL id) down to one row per
-- progress key, and ON CONFLICT DO UPDATE keeps the earliest non-null
-- timestamp. Runs fine on an empty DB (0 rows).
-- ---------------------------------------------------------------------

-- anime_episodes -> user_episode_progress
-- media_items may be keyed by provider='mal' + external_id, or carry the
-- MAL id in the dedicated mal_id column (Shikimori-sourced anime).
INSERT INTO user_episode_progress
    (user_id, provider, external_id, episode_number, watched, watched_at)
SELECT DISTINCT ON (te.user_id, ae.provider, ae.external_id, ae.episode_number)
    te.user_id,
    ae.provider,
    ae.external_id,
    ae.episode_number,
    TRUE,
    ae.watched_at
FROM anime_episodes ae
JOIN media_items mi
    ON (mi.provider = 'mal' AND mi.external_id = ae.external_id)
    OR (mi.mal_id::text = ae.external_id)
JOIN tracking_entries te ON te.media_id = mi.id
WHERE ae.watched = TRUE
ORDER BY
    te.user_id,
    ae.provider,
    ae.external_id,
    ae.episode_number,
    ae.watched_at DESC NULLS LAST
ON CONFLICT (user_id, provider, external_id, episode_number) DO UPDATE
SET watched = TRUE,
    watched_at = COALESCE(EXCLUDED.watched_at, user_episode_progress.watched_at);

-- series_chapters -> user_chapter_progress
INSERT INTO user_chapter_progress
    (user_id, provider, external_id, chapter_number, read, read_at)
SELECT DISTINCT ON (te.user_id, sc.provider, sc.external_id, sc.chapter_number)
    te.user_id,
    sc.provider,
    sc.external_id,
    sc.chapter_number,
    TRUE,
    sc.read_at
FROM series_chapters sc
JOIN media_items mi
    ON mi.provider = sc.provider
    AND mi.external_id = sc.external_id
JOIN tracking_entries te ON te.media_id = mi.id
WHERE sc.read = TRUE
ORDER BY
    te.user_id,
    sc.provider,
    sc.external_id,
    sc.chapter_number,
    sc.read_at DESC NULLS LAST
ON CONFLICT (user_id, provider, external_id, chapter_number) DO UPDATE
SET read = TRUE,
    read_at = COALESCE(EXCLUDED.read_at, user_chapter_progress.read_at);

-- tmdb_episodes -> user_tmdb_episode_progress
INSERT INTO user_tmdb_episode_progress
    (user_id, external_id, season_number, episode_number, watched, watched_at)
SELECT DISTINCT ON (te.user_id, tme.external_id, tme.season_number, tme.episode_number)
    te.user_id,
    tme.external_id,
    tme.season_number,
    tme.episode_number,
    TRUE,
    tme.watched_at
FROM tmdb_episodes tme
JOIN media_items mi
    ON mi.provider = 'tmdb'
    AND mi.external_id = tme.external_id
JOIN tracking_entries te ON te.media_id = mi.id
WHERE tme.watched = TRUE
ORDER BY
    te.user_id,
    tme.external_id,
    tme.season_number,
    tme.episode_number,
    tme.watched_at DESC NULLS LAST
ON CONFLICT (user_id, external_id, season_number, episode_number) DO UPDATE
SET watched = TRUE,
    watched_at = COALESCE(EXCLUDED.watched_at, user_tmdb_episode_progress.watched_at);
