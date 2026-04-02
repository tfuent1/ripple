-- v1 — initial schema
-- Never modify this file. Add new migrations as 002_*, 003_*, etc.

CREATE TABLE IF NOT EXISTS bundles (
    id               TEXT    PRIMARY KEY,
    destination      TEXT    NOT NULL,
    dest_pubkey      BLOB,
    priority         INTEGER NOT NULL,
    expires_at       INTEGER,
    delivered        INTEGER NOT NULL DEFAULT 0,
    displayed        INTEGER NOT NULL DEFAULT 0,
    spray_remaining  INTEGER,
    raw              BLOB    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_bundles_dest_pubkey
    ON bundles (dest_pubkey) WHERE dest_pubkey IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_bundles_expires_at
    ON bundles (expires_at) WHERE expires_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS encounters (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    peer_pubkey BLOB    NOT NULL,
    transport   INTEGER NOT NULL,
    rssi        INTEGER NOT NULL,
    seen_at     INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_encounters_seen_at
    ON encounters (seen_at);
