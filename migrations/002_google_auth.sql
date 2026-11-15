-- Phaenna In-App Google Identity & Account Schema

CREATE TABLE IF NOT EXISTS accounts (
    account_id     TEXT PRIMARY KEY,
    email          TEXT NOT NULL,
    display_name   TEXT NOT NULL,
    channel_id     TEXT,
    channel_handle TEXT,
    avatar_url     TEXT,
    auth_kind      TEXT NOT NULL,
    is_active      INTEGER NOT NULL DEFAULT 0,
    added_at       INTEGER NOT NULL,
    last_refreshed INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_accounts_active ON accounts(is_active);
