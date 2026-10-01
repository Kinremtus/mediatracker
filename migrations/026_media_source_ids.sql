-- 026_media_source_ids.sql
-- Layer 3: per-source id binding for a skeleton media item.
-- A row maps a skeleton (provider, external_id) -- e.g. MangaUpdates -- to the
-- title id inside ONE original-language source (kakao / mangaplus / syosetu).
-- chapter_enrich looks up this row before calling a provider; absent => skip.
-- Free-form source_id + source means a new integration needs no ALTER TABLE.

CREATE TABLE media_source_ids (
    id          BIGSERIAL PRIMARY KEY,
    provider    TEXT        NOT NULL,
    external_id TEXT        NOT NULL,
    source      TEXT        NOT NULL,
    source_id   TEXT        NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT media_source_ids_unique UNIQUE (provider, external_id, source)
);

-- Enrichment/list lookup path. NB: the UNIQUE btree above already covers the
-- (provider, external_id) prefix; this index is kept explicit per the design
-- spec and is negligible on a small table.
CREATE INDEX idx_media_source_ids_provider_external
    ON media_source_ids (provider, external_id);
