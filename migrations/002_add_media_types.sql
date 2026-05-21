-- Add missing media types to CHECK constraint
ALTER TABLE media_items DROP CONSTRAINT media_items_media_type_check;

ALTER TABLE media_items ADD CONSTRAINT media_items_media_type_check CHECK (
    media_type::text = ANY (ARRAY[
        'anime', 'manga', 'manhwa', 'manhua', 'novel',
        'movie', 'series', 'game', 'book',
        'dramas', 'cartoons', 'animated-movies', 'other-comics'
    ]::text[])
);
