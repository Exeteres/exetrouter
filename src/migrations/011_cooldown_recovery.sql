-- A per-account rejection fence, independent of whether a rejection has windows.
ALTER TABLE oauth_accounts ADD COLUMN cooldown_order INTEGER NOT NULL DEFAULT 0;
