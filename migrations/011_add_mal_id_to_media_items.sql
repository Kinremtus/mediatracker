-- mal_id is needed for MAL-sourced anime episode lookups:
-- Shikimori's episode API requires Shikimori's own id, so we resolve
-- it via `GET /api/animes?mal_id={mal_id}`. The lookup key must be
-- persisted because CreateMediaItem accepts it but add_to_list would
-- otherwise lose it on INSERT.
--
-- shikimori_id was added in migration 010; this mirrors it for MAL.
ALTER TABLE media_items ADD COLUMN mal_id BIGINT;

CREATE INDEX idx_media_items_mal_id
    ON media_items(mal_id) WHERE mal_id IS NOT NULL;
