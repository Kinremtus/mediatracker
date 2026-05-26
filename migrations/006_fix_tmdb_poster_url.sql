UPDATE media_items
SET poster_url = replace(poster_url, '/tmdb-image//', '/tmdb-image/')
WHERE poster_url LIKE '/tmdb-image//%';
