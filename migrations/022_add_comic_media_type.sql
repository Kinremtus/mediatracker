-- 022_add_comic_media_type.sql
-- Западные комиксы (Marvel/DC) через Comic Vine.
-- Единица прогресса для comic — выпуски ("вып.").

ALTER TABLE media_items DROP CONSTRAINT IF EXISTS media_items_media_type_check;

ALTER TABLE media_items ADD CONSTRAINT media_items_media_type_check CHECK (
    media_type::text = ANY (ARRAY[
        'anime', 'manga', 'manhwa', 'manhua', 'novel',
        'movie', 'series', 'game', 'book',
        'dramas', 'cartoons', 'animated-movies', 'other-comics', 'comic'
    ]::text[])
);
