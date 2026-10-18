-- 032_official_meta_attempts.sql
-- Negative cache for the official-meta chain. A title with no official match
-- has no `series_chapters` rows, so the freshness gate never fired and every
-- drawer open re-walked up to nine sources (up to 20 s each). A fruitless
-- walk now records an attempt, and the gate treats a recent attempt as fresh
-- until the TTL passes; a successful apply (and every force refresh, e.g. a
-- manual bind) clears the marker.

CREATE TABLE IF NOT EXISTS official_meta_attempts (
    provider     TEXT NOT NULL,
    external_id  TEXT NOT NULL,
    attempted_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (provider, external_id)
);

CREATE INDEX IF NOT EXISTS official_meta_attempts_at_idx
    ON official_meta_attempts (attempted_at);

COMMENT ON TABLE official_meta_attempts IS
    'negative cache: the resolved key''s last fruitless official-meta walk; the freshness gate skips the chain while a row is recent';
