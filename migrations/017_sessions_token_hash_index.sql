-- 017_sessions_token_hash_index.sql
-- token_hash is the lookup key on every authenticated request
-- (SELECT ... WHERE token_hash = $1). Make it unique so a duplicate
-- cannot ever be issued/inserted, and add an expires_at index for
-- cleanup / expiry sweeps.

-- First collapse any existing exact duplicates, keeping the newest
-- session per token_hash (max created_at, tie-broken by physical ctid).
DELETE FROM sessions a
USING sessions b
WHERE a.token_hash = b.token_hash
  AND (
      a.created_at < b.created_at
      OR (a.created_at = b.created_at AND a.ctid < b.ctid)
  );

CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_token_hash
    ON sessions(token_hash);

CREATE INDEX IF NOT EXISTS idx_sessions_expires_at
    ON sessions(expires_at);
