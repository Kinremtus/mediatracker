-- 034_search_catalog_queries.sql
-- The catalog answer of a query is a SET of catalog rows, but `last_query` can
-- only name one query per row: the lookup used to approximate the answer with
-- `title ILIKE '%<whole query>%'`, which returns only titles containing the
-- entire query string. A live provider search is fuzzy — e.g.
-- "Tsuki ga Michibiku Isekai Douchuu" stored 24 manga rows and exactly ONE
-- title contains the whole phrase, so the replayed search showed 1 result
-- instead of 24.
--
-- This join table records, per (query_norm, media_type), the exact answer set.

CREATE TABLE IF NOT EXISTS search_catalog_queries (
    query_norm  TEXT NOT NULL,
    media_type  TEXT NOT NULL,
    provider    TEXT NOT NULL,
    external_id TEXT NOT NULL,
    PRIMARY KEY (query_norm, media_type, provider, external_id)
);

-- Backfill: `last_query` is the only association pre-034 rows have.
INSERT INTO search_catalog_queries (query_norm, media_type, provider, external_id)
SELECT last_query, media_type, provider, external_id FROM search_catalog
ON CONFLICT DO NOTHING;

COMMENT ON TABLE search_catalog_queries IS
    'exact answer set of a recorded search: (query_norm, media_type) -> catalog rows (provider, external_id)';
