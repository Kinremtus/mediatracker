-- 021_game_additions.sql
-- DLC / expansions ("additions") for games, cached from the game provider.
-- Mirrors the lazy DB-first pattern used for manga chapters
-- (series_chapters + /api/manga/{provider}/{external_id}/chapters).
--   provider             : 'rawg' | 'igdb'  (media_items.provider)
--   external_id          : parent game id at that provider
--   addition_external_id : DLC/expansion id at the same provider
--   kind                 : 'dlc' | 'expansion' | 'addition'
-- Fetched on demand and upserted idempotently. Rows that vanish from the
-- provider source are intentionally kept (design decision).

CREATE TABLE game_additions (
    id BIGSERIAL PRIMARY KEY,
    provider VARCHAR(20) NOT NULL,
    external_id VARCHAR(100) NOT NULL,
    addition_external_id VARCHAR(100) NOT NULL,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('dlc', 'expansion', 'addition')),
    released DATE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider, external_id, addition_external_id)
);

CREATE INDEX idx_game_additions_lookup
    ON game_additions(provider, external_id);
