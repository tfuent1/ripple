//! SQLite persistence layer for the rendezvous server.
//!
//! All database access goes through [`Db`]. The rest of the server
//! treats it as an opaque handle — no SQL outside this module.

use ripple_core::bundle::{Bundle, Destination};
use rusqlite::{params, Connection};
use thiserror::Error;

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors that can come out of the database layer.
///
/// **Rust note — `thiserror`:**
/// The `#[derive(Error)]` macro generates the `std::error::Error` impl for us.
/// `#[error("...")]` sets the human-readable message for each variant.
/// `#[from]` on a field means "auto-convert this type into DbError via `?`".
/// That's the same pattern as `StoreError` in `core/src/store.rs`.
#[derive(Debug, Error)]
pub enum DbError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("bundle parse error: {0}")]
    BundleParse(String),
}

// ── Db handle ─────────────────────────────────────────────────────────────────

/// A wrapper around a SQLite connection.
///
/// The connection is opened once at startup and shared across all request
/// handlers via `Arc<Mutex<Db>>`. The `Mutex` ensures only one handler
/// touches the connection at a time — SQLite in WAL mode is fine with
/// concurrent reads in theory, but `rusqlite::Connection` is not `Send`
/// on its own, so the Mutex is required regardless.
///
/// **Rust note — `Send`:**
/// Rust tracks which types are safe to move between threads. `Connection`
/// is not `Send` by itself because SQLite's C library has thread-local
/// state. Wrapping it in `Mutex<T>` makes the whole thing `Send + Sync`
/// because the lock guarantees only one thread runs at a time.
pub struct Db {
    conn: Connection,
}

impl Db {
    /// Open (or create) the SQLite database at `path` and run migrations.
    ///
    /// Pass `":memory:"` in tests for an in-process throwaway database.
    pub fn open(path: &str) -> Result<Self, DbError> {
        let conn = Connection::open(path)?;

        // WAL mode — better concurrent read performance and crash safety.
        // Same setting used in ripple-core's Store.
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        migrate(&conn)?;

        Ok(Self { conn })
    }

    /// Insert a bundle. Idempotent — duplicate IDs are silently ignored.
    ///
    /// Returns `true` if the bundle was newly inserted, `false` if it was
    /// already present (INSERT OR IGNORE).
    pub fn insert_bundle(&self, raw: &[u8]) -> Result<bool, DbError> {
        let bundle = Bundle::from_bytes(raw).map_err(|e| DbError::BundleParse(e.to_string()))?;
        bundle.verify().map_err(|e| DbError::BundleParse(e.to_string()))?;

        let dest_pubkey_hex = match &bundle.destination {
            Destination::Peer(pk) => hex::encode(pk),
            Destination::Broadcast => "broadcast".to_string(),
        };

        let rows_changed = self.conn.execute(
            "INSERT OR IGNORE INTO bundles (id, dest_pubkey, raw, expires_at, created_at)
             VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
            params![
                bundle.id.to_string(),
                dest_pubkey_hex,
                raw,
                bundle.expires_at,
            ],
        )?;

        Ok(rows_changed > 0)
    }

    /// Return all non-expired bundles destined for `pubkey_hex`.
    ///
    /// Expired bundles are deleted before the query runs — inbox polls
    /// are a natural housekeeping trigger, same as in ripple-core's Store.
    pub fn bundles_for_pubkey(&self, pubkey_hex: &str) -> Result<Vec<Vec<u8>>, DbError> {
        // Expire stale bundles first.
        self.conn.execute(
            "DELETE FROM bundles
             WHERE expires_at IS NOT NULL
               AND expires_at <= strftime('%s','now')",
            [],
        )?;

        let mut stmt = self
            .conn
            .prepare("SELECT raw FROM bundles WHERE dest_pubkey = ?1")?;

        // `query_map` gives us an iterator of `Result<T, rusqlite::Error>`.
        // `.collect::<Result<Vec<_>, _>>()` turns that into a single
        // `Result<Vec<T>, rusqlite::Error>` — if any row fails, the whole
        // thing fails. The `?` at the end propagates that outward as DbError
        // via the `#[from]` impl we declared above.
        let rows = stmt
            .query_map(params![pubkey_hex], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
    }

    /// Delete a bundle by its UUID string. Used to acknowledge delivery.
    pub fn delete_bundle(&self, bundle_id: &str) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM bundles WHERE id = ?1", params![bundle_id])?;
        Ok(())
    }
}

// ── Schema migrations ─────────────────────────────────────────────────────────

/// Run schema migrations. Safe to call on an existing database —
/// `CREATE TABLE IF NOT EXISTS` is idempotent.
fn migrate(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bundles (
            id          TEXT    PRIMARY KEY,
            dest_pubkey TEXT    NOT NULL,
            raw         BLOB    NOT NULL,
            expires_at  INTEGER,
            created_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_dest     ON bundles(dest_pubkey);
        CREATE INDEX IF NOT EXISTS idx_expires  ON bundles(expires_at);",
    )?;
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal valid bundle for testing.
    /// We need a real signed bundle because `insert_bundle` calls
    /// `Bundle::from_bytes` to extract the destination pubkey.
    fn make_test_bundle_bytes(dest_x25519: [u8; 32]) -> Vec<u8> {
        use ripple_core::bundle::{BundleBuilder, Destination, Priority};
        use ripple_core::crypto::Identity;

        let identity = Identity::generate();
        BundleBuilder::new(Destination::Peer(dest_x25519), Priority::Normal)
            .payload(b"test message".to_vec())
            .build(&identity, 9_999_999_999)
            .unwrap()
            .to_bytes()
            .unwrap()
    }

    #[test]
    fn open_in_memory() {
        let db = Db::open(":memory:").unwrap();
        // If we got here the schema ran clean.
        let _ = db;
    }

    #[test]
    fn insert_and_fetch() {
        let db = Db::open(":memory:").unwrap();
        let dest = [1u8; 32];
        let raw = make_test_bundle_bytes(dest);

        let inserted = db.insert_bundle(&raw).unwrap();
        assert!(inserted, "first insert should return true");

        let rows = db.bundles_for_pubkey(&hex::encode(dest)).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0], raw);
    }

    #[test]
    fn insert_idempotent() {
        let db = Db::open(":memory:").unwrap();
        let raw = make_test_bundle_bytes([2u8; 32]);

        let first = db.insert_bundle(&raw).unwrap();
        let second = db.insert_bundle(&raw).unwrap();

        assert!(first, "first insert: true");
        assert!(!second, "duplicate insert: false");
    }

    #[test]
    fn delete_bundle() {
        let db = Db::open(":memory:").unwrap();
        let dest = [3u8; 32];
        let raw = make_test_bundle_bytes(dest);

        db.insert_bundle(&raw).unwrap();

        // Parse the bundle to get its ID for the delete call.
        let bundle = Bundle::from_bytes(&raw).unwrap();
        db.delete_bundle(&bundle.id.to_string()).unwrap();

        let rows = db.bundles_for_pubkey(&hex::encode(dest)).unwrap();
        assert!(rows.is_empty(), "bundle should be gone after delete");
    }

    #[test]
    fn unknown_pubkey_returns_empty() {
        let db = Db::open(":memory:").unwrap();
        let rows = db.bundles_for_pubkey(&hex::encode([99u8; 32])).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn insert_rejects_tampered_bundle() {
        let db = Db::open(":memory:").unwrap();
        let dest = [4u8; 32];

        // Build a valid bundle then tamper with the payload after signing.
        let identity = ripple_core::crypto::Identity::generate();
        let mut bundle = ripple_core::bundle::BundleBuilder::new(
            ripple_core::bundle::Destination::Peer(dest),
            ripple_core::bundle::Priority::Normal,
        )
        .payload(b"legitimate".to_vec())
        .build(&identity, 9_999_999_999)
        .unwrap();

        bundle.payload = b"tampered".to_vec();
        let raw = bundle.to_bytes().unwrap();

        // Signature is now invalid — server should reject it.
        let result = db.insert_bundle(&raw);
        assert!(result.is_err());
    }
}
