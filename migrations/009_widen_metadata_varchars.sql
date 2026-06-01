-- Расширение VARCHAR(50) колонок до TEXT для полей метаданных.
-- MangaUpdates, MAL, Shikimori и другие провайдеры могут возвращать длинные
-- значения в status/source/rating/format_type (например, "72 Volumes (Complete)
-- 24 Combini-ban Volumes (Complete)" — 57 символов).
ALTER TABLE media_items
    ALTER COLUMN status      TYPE TEXT,
    ALTER COLUMN source      TYPE TEXT,
    ALTER COLUMN rating      TYPE TEXT,
    ALTER COLUMN format_type TYPE TEXT;
