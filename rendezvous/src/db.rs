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
        bundle
            .verify()
            .map_err(|e| DbError::BundleParse(e.to_string()))?;

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

    /// Delete all expired buncles. Called periodically by the background sweep task.
    ///
    /// SOS bundles have 'expires_at = NULL' and are never touched
    /// Returns the number of rows deleted
    pub fn expire_bundles(&self) -> Result<u32, DbError> {
        let count = self.conn.execute(
            "DELETE FROM bundles
            WHERE expires_at IS NOT NULL
            AND expires_at <= strftime('%s', 'now')",
            [],
        )?;
        Ok(count as u32)
    }

    /// Delete a bundle by its UUID string. Used to acknowledge delivery.
    pub fn delete_bundle(&self, bundle_id: &str) -> Result<(), DbError> {
        self.conn
            .execute("DELETE FROM bundles WHERE id = ?1", params![bundle_id])?;
        Ok(())
    }
}

// ── Schema migrations ─────────────────────────────────────────────────────────

/// Ordered list of SQL migrations, embedded at compile time from the
/// `rendezvous/migrations/` directory via `include_str!`.
///
/// Rules:
/// - Never modify an existing entry — it may already be applied to live databases.
/// - To add a migration: create `NNN_description.sql` in `rendezvous/migrations/`
///   and add a corresponding `include_str!` line here. The version number is
///   derived automatically from position — no manual incrementing required.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_initial_schema.sql"),
    // include_str!("../migrations/002_your_next_migration.sql"),
];

fn migrate(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;

    let current_version: usize = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;

    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let migration_version = i + 1;
        if current_version < migration_version {
            conn.execute_batch(sql)?;
            conn.execute_batch(&format!("PRAGMA user_version = {migration_version};"))?;
        }
    }

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
    fn expire_bundles_removes_stale() {
        // This test uses a bundle built with a timestamp far in the past so
        // its expires_at is already behind strftime('%s','now') at insert time.
        // We bypass insert_bundle (which would still store it) and insert
        // directly with an explicit past expires_at to simulate a stale entry.
        let db = Db::open(":memory:").unwrap();

        // Insert a row that expired at Unix timestamp 1 (Jan 1 1970 + 1s).
        db.conn
            .execute(
                "INSERT INTO bundles (id, dest_pubkey, raw, expires_at, created_at)
                 VALUES ('test-id', 'aabbcc', X'00', 1, 1)",
                [],
            )
            .unwrap();

        let deleted = db.expire_bundles().unwrap();
        assert_eq!(deleted, 1, "expired bundle should be deleted");

        // A second call on an empty table should delete nothing.
        let deleted2 = db.expire_bundles().unwrap();
        assert_eq!(deleted2, 0);
    }

    #[test]
    fn expire_bundles_preserves_sos() {
        // SOS bundles have expires_at = NULL and must survive expiry sweeps.
        let db = Db::open(":memory:").unwrap();

        db.conn
            .execute(
                "INSERT INTO bundles (id, dest_pubkey, raw, expires_at, created_at)
                 VALUES ('sos-id', 'aabbcc', X'00', NULL, 1)",
                [],
            )
            .unwrap();

        let deleted = db.expire_bundles().unwrap();
        assert_eq!(
            deleted, 0,
            "SOS bundle (expires_at = NULL) must not be deleted"
        );
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
