-- 030_search_catalog.sql
-- Local search catalog: every search result (official sources first, the
-- provider chains as fallback) is persisted here, so a repeated search is
-- served from the DB instead of re-querying five-plus upstreams. The catalog
-- NEVER deletes rows by age: a title that disappears from its source stays
-- with its last known data (manual re-fill is a separate future feature); the
-- refresh worker only upserts fresh data for stale queries.
--
-- search_catalog  - one row per (provider, external_id): the display fields
--                   the grid needs directly plus the full CreateMediaItem
--                   payload (add-form data + drawer preview fields).
-- search_queries  - the log of executed live searches: `query_norm` is the
--                   normalized form the lookup matches on, `query_text` is the
--                   original text the refresh worker re-runs against the
--                   sources. Only non-empty live answers are recorded, so a
--                   transient upstream failure can never hide a title for a
--                   whole TTL window.
-- users.default_search_type - the media type preselected in the header search
--                   selector; NULL = none (the search page then asks to pick).

CREATE TABLE IF NOT EXISTS search_catalog (
    provider      TEXT NOT NULL,
    external_id   TEXT NOT NULL,
    media_type    TEXT NOT NULL,
    title         TEXT NOT NULL,
    title_norm    TEXT NOT NULL,
    poster_url    TEXT,
    year          SMALLINT,
    score         DOUBLE PRECISION,
    is_official   BOOLEAN NOT NULL DEFAULT FALSE,
    payload       JSONB NOT NULL,
    last_query    TEXT NOT NULL,
    fetched_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, external_id)
);

CREATE INDEX IF NOT EXISTS search_catalog_type_title_idx
    ON search_catalog (media_type, title_norm);

CREATE TABLE IF NOT EXISTS search_queries (
    query_norm    TEXT NOT NULL,
    media_type    TEXT NOT NULL,
    query_text    TEXT NOT NULL,
    fetched_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (query_norm, media_type)
);

ALTER TABLE users
    ADD COLUMN IF NOT EXISTS default_search_type TEXT;

COMMENT ON TABLE search_catalog IS
    'local cache of search results (official sources + provider chains); rows are never deleted by age';
COMMENT ON TABLE search_queries IS
    'log of executed live searches; the refresh worker re-runs the oldest rows';
COMMENT ON COLUMN users.default_search_type IS
    'media type preselected in the header search selector; NULL = none';
