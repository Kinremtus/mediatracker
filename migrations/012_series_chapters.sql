-- Chapter tracking for manga/manhwa/manhua/novel/other-comics.
-- Stores under provider = 'mangaupdates', external_id = series_id.
-- chapter_number stored as integer * 10 (e.g., 105 = chapter 10.5).
-- This avoids floating-point issues while supporting fractional chapters.

CREATE TABLE series_chapters (
    id BIGSERIAL PRIMARY KEY,
    provider VARCHAR(20) NOT NULL,
    external_id VARCHAR(100) NOT NULL,
    chapter_number INTEGER NOT NULL,           -- chapter * 10 (10 = ch.1, 105 = ch.10.5)
    volume INTEGER,                            -- volume number (nullable)
    title_en TEXT,                             -- optional, filled from MangaDex later
    title_ru TEXT,
    release_date DATE,
    read BOOLEAN NOT NULL DEFAULT FALSE,
    read_at TIMESTAMPTZ,
    fetched_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (provider, external_id, chapter_number)
);

CREATE INDEX idx_series_chapters_lookup
    ON series_chapters(provider, external_id, chapter_number);

CREATE INDEX idx_series_chapters_unread
    ON series_chapters(provider, external_id)
    WHERE read = FALSE;