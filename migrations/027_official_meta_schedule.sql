-- 027_official_meta_schedule.sql
-- Free-text update schedule + exact next-update date, filled only by a bound
-- official source. Nullable: absence is the normal state.
--   update_schedule : heterogeneous per-source text, e.g. '每周四14点更新',
--                     '매주 월요일', '毎週 金曜日 19:00', '不定时更新'
--   next_update_at  : exact date when the source publishes one (Alpha Polis,
--                     Comic Walker, Manga UP! EN); NULL otherwise.
ALTER TABLE media_items
    ADD COLUMN update_schedule TEXT;

ALTER TABLE media_items
    ADD COLUMN next_update_at DATE;

COMMENT ON COLUMN media_items.update_schedule IS
    'Free-text serialization schedule as published by a bound official source';
COMMENT ON COLUMN media_items.next_update_at IS
    'Exact next chapter/episode date when a bound official source provides one';
