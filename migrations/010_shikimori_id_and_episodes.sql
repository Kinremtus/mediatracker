-- Add shikimori_id to media_items for Shikimori episode API lookups.
-- Shikimori API endpoints (/animes/{id}/episodes) require Shikimori's own
-- internal id, not the MAL id. Store both so we can fetch episode lists
-- without an extra id-conversion roundtrip.
ALTER TABLE media_items ADD COLUMN shikimori_id BIGINT;
CREATE INDEX idx_media_items_shikimori_id ON media_items(shikimori_id) WHERE shikimori_id IS NOT NULL;

-- Episode list per anime. UNIQUE (provider, external_id, episode_number)
-- makes UPSERT idempotent — re-fetching the same anime just updates titles
-- and air dates in place. watched/watched_at are added now (not used yet,
-- filled in by Stage B when we add the toggle UI).
CREATE TABLE anime_episodes (
    id BIGSERIAL PRIMARY KEY,
    provider VARCHAR(20) NOT NULL,
    external_id VARCHAR(100) NOT NULL,
    episode_number INTEGER NOT NULL,
    title_en TEXT,
    title_ru TEXT,
    title_jp TEXT,
    title_other TEXT,
    air_date DATE,
    duration_minutes INTEGER,
    watched BOOLEAN NOT NULL DEFAULT FALSE,
    watched_at TIMESTAMPTZ,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider, external_id, episode_number)
);

CREATE INDEX idx_anime_episodes_lookup
    ON anime_episodes(provider, external_id, episode_number);

CREATE INDEX idx_anime_episodes_unwatched
    ON anime_episodes(provider, external_id)
    WHERE watched = FALSE;
