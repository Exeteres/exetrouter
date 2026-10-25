-- Historical routing overrides remain for rollback only; new binaries ignore them.
CREATE TABLE IF NOT EXISTS oauth_quota_absences (
 account_id INTEGER NOT NULL REFERENCES oauth_accounts(id) ON DELETE CASCADE,
 kind TEXT NOT NULL CHECK(kind IN ('primary','secondary')),
 request_order INTEGER NOT NULL,
 PRIMARY KEY(account_id,kind)
);
CREATE TABLE IF NOT EXISTS account_routing_metadata (
 account_id INTEGER PRIMARY KEY REFERENCES oauth_accounts(id) ON DELETE CASCADE,
 generation INTEGER NOT NULL,
 request_order INTEGER NOT NULL,
 observed_at INTEGER NOT NULL,
 short_window INTEGER CHECK(short_window IN (0,1)),
 credits TEXT
);
-- Digests, never upstream credit IDs. A claim survives an uncertain POST/restart.
CREATE TABLE IF NOT EXISTS automatic_reset_attempts (
 cycle_digest BLOB PRIMARY KEY CHECK(length(cycle_digest)=32),
 credit_digest BLOB NOT NULL CHECK(length(credit_digest)=32),
 account_id INTEGER NOT NULL REFERENCES oauth_accounts(id) ON DELETE CASCADE,
 attempted_at INTEGER NOT NULL,
 quota_snapshot TEXT NOT NULL,
 salvage_until INTEGER NOT NULL DEFAULT 0,
 recovered_at INTEGER,
 status TEXT NOT NULL CHECK(status IN ('submitted','reset','already_redeemed','nothing_to_reset','no_credit','rejected','unknown','recovered'))
);
CREATE INDEX IF NOT EXISTS automatic_reset_account ON automatic_reset_attempts(account_id);
CREATE INDEX IF NOT EXISTS automatic_reset_credit ON automatic_reset_attempts(credit_digest);
CREATE TABLE IF NOT EXISTS automatic_reset_observations (
 account_id INTEGER PRIMARY KEY REFERENCES oauth_accounts(id) ON DELETE CASCADE,
 request_order INTEGER NOT NULL,
 observation TEXT NOT NULL
);
