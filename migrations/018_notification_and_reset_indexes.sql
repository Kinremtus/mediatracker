-- 018_notification_and_reset_indexes.sql
-- Enforce dedup at the DB level for two idempotency keys that were
-- previously only guarded in application code.

-- notification_log: one notification per user + media + episode.
-- Collapse duplicates first, keeping the newest row per key
-- (max created_at, tie-broken by physical ctid).
DELETE FROM notification_log a
USING notification_log b
WHERE a.user_id = b.user_id
  AND a.provider = b.provider
  AND a.external_id = b.external_id
  AND a.episode_number = b.episode_number
  AND (
      a.created_at < b.created_at
      OR (a.created_at = b.created_at AND a.ctid < b.ctid)
  );

CREATE UNIQUE INDEX IF NOT EXISTS uq_notification_log
    ON notification_log(user_id, provider, external_id, episode_number);

-- password_reset_tokens: token_hash is the lookup key and must be unique.
CREATE UNIQUE INDEX IF NOT EXISTS uq_password_reset_tokens_hash
    ON password_reset_tokens(token_hash);

CREATE INDEX IF NOT EXISTS idx_password_reset_tokens_expires
    ON password_reset_tokens(expires_at);
