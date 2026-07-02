CREATE TABLE tmdb_episodes (
    id BIGSERIAL PRIMARY KEY,
    external_id VARCHAR(100) NOT NULL,
    season_number INTEGER NOT NULL,
    episode_number INTEGER NOT NULL,
    title TEXT,
    overview TEXT,
    still_path TEXT,
    air_date DATE,
    duration_minutes INTEGER,
    watched BOOLEAN NOT NULL DEFAULT FALSE,
    watched_at TIMESTAMPTZ,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (external_id, season_number, episode_number)
);

CREATE INDEX idx_tmdb_episodes_lookup
    ON tmdb_episodes(external_id, season_number, episode_number);

CREATE INDEX idx_tmdb_episodes_unwatched
    ON tmdb_episodes(external_id, season_number)
    WHERE watched = FALSE;
