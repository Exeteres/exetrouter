-- Fixed feature keys: quota/availability here must never exhaust ordinary Codex.
CREATE TABLE IF NOT EXISTS native_feature_pauses (
    account_id INTEGER NOT NULL REFERENCES oauth_accounts(id),
    feature TEXT NOT NULL CHECK(feature IN ('images','search')),
    generation INTEGER NOT NULL,
    retry_at INTEGER NOT NULL,
    last_order INTEGER NOT NULL,
    kind TEXT NOT NULL CHECK(kind IN ('quota','backoff')),
    PRIMARY KEY(account_id,feature)
);
-- Native search references depend on backend-owned session state. Retain only a
-- keyed session digest, never queries, pages, reference strings or results.
CREATE TABLE IF NOT EXISTS native_search_sessions (
    user_id INTEGER NOT NULL REFERENCES users(id),
    digest BLOB NOT NULL CHECK(length(digest)=32),
    account_id INTEGER NOT NULL REFERENCES oauth_accounts(id),
    PRIMARY KEY(user_id,digest)
);
