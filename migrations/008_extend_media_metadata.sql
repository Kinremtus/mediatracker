-- Расширение метаданных медиа (Type, Episodes/Chapters/Volumes/Pages/Hours, Aired, Premiered,
-- Broadcast, Producers, Licensors, Studios, Authors/Artists, Publishers, Source, Duration, Rating,
-- Genres/Themes/Demographics/Categories, etc.)
ALTER TABLE media_items
    ADD COLUMN format_type VARCHAR(20),
    ADD COLUMN details JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN chapters INTEGER,
    ADD COLUMN volumes INTEGER,
    ADD COLUMN pages INTEGER,
    ADD COLUMN runtime_minutes INTEGER,
    ADD COLUMN playtime_hours INTEGER,
    ADD COLUMN year SMALLINT,
    ADD COLUMN aired_from DATE,
    ADD COLUMN aired_to DATE,
    ADD COLUMN premiered_season VARCHAR(10),
    ADD COLUMN premiered_year SMALLINT,
    ADD COLUMN broadcast TEXT,
    ADD COLUMN completed BOOLEAN,
    ADD COLUMN licensed BOOLEAN,
    ADD COLUMN source VARCHAR(50),
    ADD COLUMN duration TEXT,
    ADD COLUMN rating VARCHAR(50),
    ADD COLUMN rating_votes INTEGER,
    ADD COLUMN authors TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN artists TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN studios TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN producers TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN licensors TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN publishers TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN serialized_in TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN networks TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN platforms TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN genres TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN themes TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN demographics TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN categories TEXT[] NOT NULL DEFAULT '{}';

-- GIN-индексы для будущей фильтрации по массивам
CREATE INDEX IF NOT EXISTS idx_media_genres_gin ON media_items USING gin(genres);
CREATE INDEX IF NOT EXISTS idx_media_themes_gin ON media_items USING gin(themes);
CREATE INDEX IF NOT EXISTS idx_media_demographics_gin ON media_items USING gin(demographics);
CREATE INDEX IF NOT EXISTS idx_media_categories_gin ON media_items USING gin(categories);
CREATE INDEX IF NOT EXISTS idx_media_studios_gin ON media_items USING gin(studios);
CREATE INDEX IF NOT EXISTS idx_media_producers_gin ON media_items USING gin(producers);
CREATE INDEX IF NOT EXISTS idx_media_authors_gin ON media_items USING gin(authors);
CREATE INDEX IF NOT EXISTS idx_media_artists_gin ON media_items USING gin(artists);
CREATE INDEX IF NOT EXISTS idx_media_publishers_gin ON media_items USING gin(publishers);
CREATE INDEX IF NOT EXISTS idx_media_serialized_in_gin ON media_items USING gin(serialized_in);
CREATE INDEX IF NOT EXISTS idx_media_networks_gin ON media_items USING gin(networks);
CREATE INDEX IF NOT EXISTS idx_media_platforms_gin ON media_items USING gin(platforms);
