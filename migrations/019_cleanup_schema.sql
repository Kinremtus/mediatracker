-- P3 cleanup:
--   * `external_mappings` was never written by the application (only deleted
--     from, in the account-deletion path), so drop the table.
--   * 007 created `users.telegram_notifications_enabled` as a nullable boolean,
--     while the application decodes it as a plain `bool`; make the column match.

DROP TABLE IF EXISTS external_mappings;

ALTER TABLE users
    ALTER COLUMN telegram_notifications_enabled SET DEFAULT false;

UPDATE users
    SET telegram_notifications_enabled = false
    WHERE telegram_notifications_enabled IS NULL;

ALTER TABLE users
    ALTER COLUMN telegram_notifications_enabled SET NOT NULL;
