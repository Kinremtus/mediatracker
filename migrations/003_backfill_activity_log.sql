-- Backfill journal from current lists (skipped rows already logged after deploy)
INSERT INTO activity_log (user_id, action, media_id, created_at)
SELECT t.user_id, 'added', t.media_id, t.created_at
FROM tracking_entries t
WHERE NOT EXISTS (
    SELECT 1 FROM activity_log a
    WHERE a.user_id = t.user_id
      AND a.media_id = t.media_id
      AND a.action = 'added'
);

INSERT INTO activity_log (user_id, action, media_id, created_at)
SELECT t.user_id, 'updated', t.media_id, t.updated_at
FROM tracking_entries t
WHERE t.updated_at > t.created_at
  AND NOT EXISTS (
    SELECT 1 FROM activity_log a
    WHERE a.user_id = t.user_id
      AND a.media_id = t.media_id
      AND a.action = 'updated'
);
