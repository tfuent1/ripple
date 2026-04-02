-- v1 — initial schema
-- Never modify this file. Add new migrations as 002_*, 003_*, etc.

CREATE TABLE IF NOT EXISTS bundles (
    id          TEXT    PRIMARY KEY,
    dest_pubkey TEXT    NOT NULL,
    raw         BLOB    NOT NULL,
    expires_at  INTEGER,
    created_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dest    ON bundles(dest_pubkey);
CREATE INDEX IF NOT EXISTS idx_expires ON bundles(expires_at);
