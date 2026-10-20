-- 033_search_queries_fetched_at_index.sql
-- The catalog refresh worker picks the oldest queries with
-- `ORDER BY fetched_at ASC`; without an index that is a full scan + sort on
-- every cycle. Small today, but the table grows with every distinct search.

CREATE INDEX IF NOT EXISTS search_queries_fetched_at_idx
    ON search_queries (fetched_at);
