use crate::bundle::Bundle;
use crate::peer::Transport;
use rusqlite::{params, Connection};
use thiserror::Error;
use uuid::Uuid;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("bundle serialization error: {0}")]
    Serialization(#[from] rmp_serde::encode::Error),

    #[error("bundle deserialization error: {0}")]
    Deserialization(#[from] rmp_serde::decode::Error),

    #[error("bundle error: {0}")]
    Bundle(#[from] crate::bundle::BundleError),
}

// ── Encounter record ──────────────────────────────────────────────────────────

/// A logged peer encounter returned by `recent_encounters`.
#[derive(Debug, Clone)]
pub struct Encounter {
    pub peer_pubkey: [u8; 32],
    pub transport: Transport,
    pub rssi: i32,
    pub seen_at: i64,
}

// ── Store ─────────────────────────────────────────────────────────────────────

/// Persistent storage for bundles and peer encounter history.
///
/// Wraps a SQLite connection. One `Store` per process — the connection is not
/// `Clone` or `Send` by default, which is fine for Phase 1's single-threaded CLI.
pub struct Store {
    conn: Connection,
}

/// Ordered list of SQL migrations, embedded at compile time from the
/// `core/migrations/` directory via `include_str!`.
///
/// Rules:
/// - Never modify an existing entry — it may already be applied to live databases.
/// - To add a migration: create `NNN_description.sql` in `core/migrations/`
///   and add a corresponding `include_str!` line here. The version number is
///   derived automatically from position — no manual incrementing required.
const MIGRATIONS: &[&str] = &[
    include_str!("../migrations/001_initial_schema.sql"),
    include_str!("../migrations/002_add_submitted_column.sql"),
];

impl Store {
    /// Open (or create) the SQLite database at `db_path`.
    /// Pass `":memory:"` in tests for a fast, isolated in-memory database.
    ///
    /// **Rust note:** `&str` here is a string slice — a borrowed reference to
    /// string data. We don't need to own the string, just read it to open the
    /// connection, so borrowing is the right choice.
    pub fn new(db_path: &str) -> Result<Self, StoreError> {
        let conn = Connection::open(db_path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), StoreError> {
        self.conn.execute_batch("PRAGMA journal_mode = WAL;")?;

        let current_version: usize = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;

        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let migration_version = i + 1;
            if current_version < migration_version {
                self.conn.execute_batch(sql)?;
                self.conn
                    .execute_batch(&format!("PRAGMA user_version = {migration_version};"))?;
            }
        }

        Ok(())
    }

    // ── Bundle operations ─────────────────────────────────────────────────────

    /// Persist a bundle. Silently ignores duplicate IDs (idempotent via INSERT OR IGNORE).
    ///
    /// **Caller is responsible for signature verification.** This method does
    /// not call `bundle.verify()`. For locally-created bundles this is correct —
    /// the builder just signed it. For bundles received from peers, callers must
    /// verify before inserting. `Router::on_bundle_received` does this; direct
    /// store callers must do it themselves.
    ///
    /// We serialize the entire bundle to MessagePack and store it in `raw`.
    /// The other columns are extracted fields used for efficient querying
    /// without deserializing the full bundle each time.
    pub fn insert_bundle(&self, bundle: &Bundle) -> Result<(), StoreError> {
        use crate::bundle::Destination;

        let id = bundle.id.to_string();
        let (destination, dest_pubkey): (&str, Option<&[u8]>) = match &bundle.destination {
            Destination::Peer(pk) => ("peer", Some(pk.as_slice())),
            Destination::Broadcast => ("broadcast", None),
        };
        let priority = bundle.priority as u8;
        let raw = bundle.to_bytes()?;

        let spray_remaining: Option<u8> = bundle.priority.spray_count();

        self.conn.execute(
            "INSERT OR IGNORE INTO bundles
                (id, destination, dest_pubkey, priority, expires_at, spray_remaining, raw)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                destination,
                dest_pubkey,
                priority,
                bundle.expires_at,
                spray_remaining,
                raw
            ],
        )?;
        Ok(())
    }

    /// Fetch a single bundle by ID. Returns `None` if not found or already delivered.
    ///
    /// **Rust note:** `Option<T>` is Rust's null-safe type. Instead of returning
    /// null and potentially crashing later, you're forced to handle both cases —
    /// `Some(bundle)` or `None` — at the call site.
    pub fn get_bundle(&self, id: Uuid) -> Result<Option<Bundle>, StoreError> {
        let id_str = id.to_string();
        let mut stmt = self
            .conn
            .prepare("SELECT raw FROM bundles WHERE id = ?1 AND delivered = 0")?;

        // `query_row` returns `Err(QueryReturnedNoRows)` when nothing is found.
        // We convert that specific error into `Ok(None)` — anything else propagates.
        let result = stmt.query_row(params![id_str], |row| {
            let raw: Vec<u8> = row.get(0)?;
            Ok(raw)
        });

        match result {
            Ok(raw) => Ok(Some(Bundle::from_bytes(&raw)?)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    /// Return all undelivered bundles addressed to `peer_pubkey`.
    /// Used when a peer is encountered to determine what to send them.
    ///
    /// **Rust note:** The return type `Result<Vec<Bundle>, StoreError>` means:
    /// success gives you a vector (growable list) of Bundles, failure gives a
    /// StoreError. In PHP this would just be an array or an exception — Rust
    /// makes both possibilities explicit in the type signature.
    pub fn bundles_for_peer(&self, peer_pubkey: &[u8; 32]) -> Result<Vec<Bundle>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT raw FROM bundles
             WHERE dest_pubkey = ?1 AND delivered = 0",
        )?;

        // `query_map` iterates over rows, applying a closure to each.
        // The `collect()` at the end gathers all the Results into one Result<Vec<_>>.
        // If any row fails to deserialize, the whole collection fails.
        let bundles = stmt
            .query_map(params![peer_pubkey.as_slice()], |row| {
                let raw: Vec<u8> = row.get(0)?;
                Ok(raw)
            })?
            .map(|raw_result| {
                let raw = raw_result.map_err(StoreError::Db)?;
                Bundle::from_bytes(&raw).map_err(StoreError::Bundle)
            })
            .collect::<Result<Vec<Bundle>, StoreError>>()?;

        Ok(bundles)
    }

    /// Return all bundles not yet submitted to the rendezvous server.
    ///
    /// "Submitted" means we POSTed the bundle to the relay. This is
    /// distinct from "delivered" (the recipient processed the bundle).
    /// A bundle moves through: unsubmitted → submitted → delivered.
    ///
    /// In Phase 1 we relay everything; Phase 3 will add transport-aware
    /// filtering so we skip bundles already synced over BLE.
    pub fn all_pending_submission(&self) -> Result<Vec<Bundle>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT raw FROM bundles WHERE submitted = 0 AND delivered = 0")?;

        let bundles = stmt
            .query_map([], |row| {
                let raw: Vec<u8> = row.get(0)?;
                Ok(raw)
            })?
            .map(|raw_result| {
                let raw = raw_result.map_err(StoreError::Db)?;
                Bundle::from_bytes(&raw).map_err(StoreError::Bundle)
            })
            .collect::<Result<Vec<Bundle>, StoreError>>()?;

        Ok(bundles)
    }

    /// Mark a bundle as submitted to the rendezvous server.
    /// It will no longer appear in `all_pending_submission`.
    pub fn mark_submitted(&self, id: Uuid) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE bundles SET submitted = 1 WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Mark a bundle as delivered so it is no longer returned by queries.
    /// We keep the row rather than deleting it so expiry logic stays simple.
    pub fn mark_delivered(&self, id: Uuid) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE bundles SET delivered = 1 WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Mark a bundle as displayed (printed to the user).
    /// `unread_count` will no longer include this bundle.
    pub fn mark_displayed(&self, id: Uuid) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE bundles SET displayed = 1 WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Count peer bundles that have been delivered (acked) but not yet
    /// displayed to the user. Used by `ripple status`.
    ///
    /// Only counts `destination = 'peer'` rows — broadcast bundles are
    /// not personal messages and don't count as unread.
    ///
    /// **Rust note:** `query_row` is used here instead of `query_map`
    /// because we expect exactly one row back (a COUNT). The closure
    /// receives that single row and extracts column 0 as a u32.
    pub fn unread_count(&self) -> Result<u32, StoreError> {
        let count = self.conn.query_row(
            "SELECT COUNT(*) FROM bundles
             WHERE delivered = 1 AND displayed = 0 AND destination = 'peer'",
            [],
            |row| row.get::<_, u32>(0),
        )?;
        Ok(count)
    }

    /// Delete all bundles whose `expires_at` is in the past.
    /// SOS bundles have `expires_at = NULL` and are never deleted here.
    /// Returns the number of bundles deleted.
    ///
    /// Called from `mesh_tick` in routing.rs (Milestone 1.4).
    pub fn expire_bundles(&self, now: i64) -> Result<u32, StoreError> {
        let count = self.conn.execute(
            "DELETE FROM bundles WHERE expires_at IS NOT NULL AND expires_at <= ?1",
            params![now],
        )?;
        // `execute` returns the number of rows affected as usize.
        // We cast to u32 — safe because we'll never delete 4 billion rows.
        Ok(count as u32)
    }

    /// Read the current spray_remaining count without modifying it.
    /// Returns `None` if the bundle uses epidemic routing or wasn't found.
    pub fn spray_remaining(&self, id: Uuid) -> Result<Option<u8>, StoreError> {
        let mut stmt = self
            .conn
            .prepare("SELECT spray_remaining FROM bundles WHERE id = ?1")?;

        let result = stmt.query_row(params![id.to_string()], |row| row.get::<_, Option<u8>>(0));

        match result {
            Ok(val) => Ok(val),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    /// Decrement the spray count for a bundle by 1.
    ///
    /// Returns the new spray_remaining value, or None if the bundle uses
    /// epidemic routing (SOS) or was not found.
    ///
    /// When spray_remaining reaches 0, the bundle enters the Waiting phase —
    /// the router stops actively spraying it and waits for a direct encounter
    /// with the destination peer.
    pub fn decrement_spray(&self, id: Uuid) -> Result<Option<u8>, StoreError> {
        self.conn.execute(
            "UPDATE bundles
             SET spray_remaining = spray_remaining - 1
             WHERE id = ?1
               AND spray_remaining IS NOT NULL
               AND spray_remaining > 0",
            params![id.to_string()],
        )?;

        let mut stmt = self
            .conn
            .prepare("SELECT spray_remaining FROM bundles WHERE id = ?1")?;

        let result = stmt.query_row(params![id.to_string()], |row| row.get::<_, Option<u8>>(0));

        match result {
            Ok(val) => Ok(val),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Db(e)),
        }
    }

    // ── Encounter operations ──────────────────────────────────────────────────

    /// Record a peer encounter. Called every time a peer is seen on any transport.
    /// `transport` is a transport type code (to be defined in peer.rs).
    /// `rssi` is signal strength in dBm (negative integer, e.g. -65).
    pub fn log_encounter(
        &self,
        peer_pubkey: &[u8; 32],
        transport: u8,
        rssi: i32,
        now: i64,
    ) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO encounters (peer_pubkey, transport, rssi, seen_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![peer_pubkey.as_slice(), transport, rssi, now],
        )?;
        Ok(())
    }

    /// Return all encounters logged since `since` (Unix timestamp seconds).
    /// Used by the PRoPHET routing layer (Milestone 1.4) to score peers.
    pub fn recent_encounters(&self, since: i64) -> Result<Vec<Encounter>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT peer_pubkey, transport, rssi, seen_at
             FROM encounters
             WHERE seen_at >= ?1
             ORDER BY seen_at DESC",
        )?;

        let encounters = stmt
            .query_map(params![since], |row| {
                // SQLite stores BLOB as Vec<u8>. We need [u8; 32].
                // `try_into()` performs the conversion — it fails if the length
                // isn't exactly 32, which would mean corrupt data.
                let raw_key: Vec<u8> = row.get(0)?;
                let peer_pubkey: [u8; 32] = raw_key.try_into().map_err(|_| {
                    rusqlite::Error::InvalidColumnType(
                        0,
                        "peer_pubkey".to_string(),
                        rusqlite::types::Type::Blob,
                    )
                })?;
                let transport_raw: u8 = row.get(1)?;
                Ok(Encounter {
                    peer_pubkey,
                    transport: Transport::from_u8(transport_raw).unwrap_or(Transport::Internet),
                    rssi: row.get(2)?,
                    seen_at: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<Encounter>, rusqlite::Error>>()?;

        Ok(encounters)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{BundleBuilder, Destination, Priority};
    use crate::crypto::Identity;
    use crate::peer::Transport;

    const NOW: i64 = 1_700_000_000;

    /// Helper — open a fresh in-memory store for each test.
    /// `:memory:` means the DB lives only for the lifetime of this connection.
    fn test_store() -> Store {
        Store::new(":memory:").unwrap()
    }

    #[test]
    fn test_insert_and_get_bundle() {
        let store = test_store();
        let identity = Identity::generate();

        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"hello store".to_vec())
            .build(&identity, NOW)
            .unwrap();

        let id = bundle.id;
        store.insert_bundle(&bundle).unwrap();

        let retrieved = store.get_bundle(id).unwrap().unwrap();
        assert_eq!(retrieved.id, id);
        assert_eq!(retrieved.payload, bundle.payload);
    }

    #[test]
    fn test_get_missing_bundle_returns_none() {
        let store = test_store();
        let result = store.get_bundle(Uuid::new_v4()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_mark_delivered_hides_bundle() {
        let store = test_store();
        let identity = Identity::generate();

        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"deliver me".to_vec())
            .build(&identity, NOW)
            .unwrap();

        let id = bundle.id;
        store.insert_bundle(&bundle).unwrap();
        store.mark_delivered(id).unwrap();

        // Should be gone from queries after delivery.
        assert!(store.get_bundle(id).unwrap().is_none());
    }

    #[test]
    fn test_bundles_for_peer() {
        let store = test_store();
        let alice = Identity::generate();
        let bob = Identity::generate();

        // One bundle for Bob, one broadcast.
        let for_bob =
            BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
                .payload(b"hey bob".to_vec())
                .build(&alice, NOW)
                .unwrap();

        let broadcast = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"everyone".to_vec())
            .build(&alice, NOW)
            .unwrap();

        store.insert_bundle(&for_bob).unwrap();
        store.insert_bundle(&broadcast).unwrap();

        let bobs_bundles = store.bundles_for_peer(&bob.x25519_public_key()).unwrap();
        assert_eq!(bobs_bundles.len(), 1);
        assert_eq!(bobs_bundles[0].id, for_bob.id);
    }

    #[test]
    fn test_expire_bundles() {
        let store = test_store();
        let identity = Identity::generate();

        // Normal bundle — expires after 24h.
        let normal = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"expires".to_vec())
            .build(&identity, NOW)
            .unwrap();

        // SOS bundle — never expires.
        let sos = BundleBuilder::new(Destination::Broadcast, Priority::Sos)
            .payload(b"mayday".to_vec())
            .build(&identity, NOW)
            .unwrap();

        let normal_id = normal.id;
        let sos_id = sos.id;

        store.insert_bundle(&normal).unwrap();
        store.insert_bundle(&sos).unwrap();

        // Travel to the future — past the normal bundle's TTL.
        let future = NOW + 25 * 3600;
        let deleted = store.expire_bundles(future).unwrap();

        assert_eq!(deleted, 1);
        assert!(store.get_bundle(normal_id).unwrap().is_none());
        assert!(store.get_bundle(sos_id).unwrap().is_some()); // SOS survives
    }

    #[test]
    fn test_log_and_retrieve_encounters() {
        let store = test_store();
        let peer = Identity::generate();
        let pubkey = peer.x25519_public_key();

        store.log_encounter(&pubkey, 0, -65, NOW).unwrap();
        store.log_encounter(&pubkey, 0, -70, NOW + 60).unwrap();

        let encounters = store.recent_encounters(NOW - 1).unwrap();
        assert_eq!(encounters.len(), 2);
        assert_eq!(encounters[0].peer_pubkey, pubkey);
        assert_eq!(encounters[0].transport, Transport::Ble);
        assert_eq!(encounters[0].rssi, -70); // most recent first
    }

    #[test]
    fn test_recent_encounters_since_filter() {
        let store = test_store();
        let peer = Identity::generate();
        let pubkey = peer.x25519_public_key();

        store.log_encounter(&pubkey, 0, -65, NOW).unwrap();
        store.log_encounter(&pubkey, 0, -70, NOW + 3600).unwrap();

        // Only ask for encounters after the first one.
        let encounters = store.recent_encounters(NOW + 1).unwrap();
        assert_eq!(encounters.len(), 1);
        assert_eq!(encounters[0].seen_at, NOW + 3600);
    }

    #[test]
    fn test_spray_remaining_initialized() {
        let store = test_store();
        let identity = Identity::generate();

        let normal = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"spray me".to_vec())
            .build(&identity, NOW)
            .unwrap();

        let sos = BundleBuilder::new(Destination::Broadcast, Priority::Sos)
            .payload(b"mayday".to_vec())
            .build(&identity, NOW)
            .unwrap();

        store.insert_bundle(&normal).unwrap();
        store.insert_bundle(&sos).unwrap();

        // Normal bundles start with spray_count() copies.
        let mut stmt = store
            .conn
            .prepare("SELECT spray_remaining FROM bundles WHERE id = ?1")
            .unwrap();

        let normal_spray: Option<u8> = stmt
            .query_row(params![normal.id.to_string()], |r| r.get(0))
            .unwrap();
        assert_eq!(normal_spray, Some(6));

        let sos_spray: Option<u8> = stmt
            .query_row(params![sos.id.to_string()], |r| r.get(0))
            .unwrap();
        assert_eq!(sos_spray, None); // epidemic — no limit
    }

    #[test]
    fn test_decrement_spray() {
        let store = test_store();
        let identity = Identity::generate();

        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"spray".to_vec())
            .build(&identity, NOW)
            .unwrap();

        let id = bundle.id;
        store.insert_bundle(&bundle).unwrap();

        // Normal starts at 6.
        let remaining = store.decrement_spray(id).unwrap();
        assert_eq!(remaining, Some(5));

        let remaining = store.decrement_spray(id).unwrap();
        assert_eq!(remaining, Some(4));
    }

    #[test]
    fn test_mark_displayed_and_unread_count() {
        let store = test_store();
        let alice = Identity::generate();
        let bob = Identity::generate();

        // Create a peer bundle (direct message) and a broadcast.
        let dm = BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
            .payload(b"hello bob".to_vec())
            .build(&alice, NOW)
            .unwrap();

        let broadcast = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"everyone".to_vec())
            .build(&alice, NOW)
            .unwrap();

        store.insert_bundle(&dm).unwrap();
        store.insert_bundle(&broadcast).unwrap();

        // Neither is delivered yet — unread count is 0.
        assert_eq!(store.unread_count().unwrap(), 0);

        // Deliver both (simulates relay ack).
        store.mark_delivered(dm.id).unwrap();
        store.mark_delivered(broadcast.id).unwrap();

        // The peer bundle is now unread; broadcast doesn't count.
        assert_eq!(store.unread_count().unwrap(), 1);

        // Display the message.
        store.mark_displayed(dm.id).unwrap();

        // Back to zero.
        assert_eq!(store.unread_count().unwrap(), 0);
    }

    #[test]
    fn test_unread_count_ignores_broadcasts() {
        let store = test_store();
        let identity = Identity::generate();

        let b = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"shout".to_vec())
            .build(&identity, NOW)
            .unwrap();

        store.insert_bundle(&b).unwrap();
        store.mark_delivered(b.id).unwrap();

        // Delivered broadcast should NOT appear as unread.
        assert_eq!(store.unread_count().unwrap(), 0);
    }

    #[test]
    fn test_all_pending_submission() {
        let store = test_store();
        let alice = Identity::generate();
        let bob = Identity::generate();

        let b1 = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"one".to_vec())
            .build(&alice, NOW)
            .unwrap();
        let b2 = BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
            .payload(b"two".to_vec())
            .build(&alice, NOW)
            .unwrap();
        let b3 = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"three".to_vec())
            .build(&alice, NOW)
            .unwrap();

        store.insert_bundle(&b1).unwrap();
        store.insert_bundle(&b2).unwrap();
        store.insert_bundle(&b3).unwrap();

        // b1 submitted to relay — no longer pending.
        store.mark_submitted(b1.id).unwrap();
        // b3 delivered (inbound processed) — also excluded.
        store.mark_delivered(b3.id).unwrap();

        // Only b2 is still pending submission.
        let pending = store.all_pending_submission().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, b2.id);
    }

    #[test]
    fn test_duplicate_insert_does_not_reset_spray_count() {
        let store = test_store();
        let identity = Identity::generate();

        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"spray test".to_vec())
            .build(&identity, NOW)
            .unwrap();

        let id = bundle.id;
        store.insert_bundle(&bundle).unwrap();

        // Decrement twice — spray_remaining goes from 6 to 4.
        store.decrement_spray(id).unwrap();
        store.decrement_spray(id).unwrap();

        let after_decrement = store.decrement_spray(id).unwrap();
        assert_eq!(after_decrement, Some(3));

        // Re-inserting the same bundle (as would happen when a relay returns it)
        // must NOT reset spray_remaining back to 6.
        store.insert_bundle(&bundle).unwrap();

        let after_reinsert = store.decrement_spray(id).unwrap();
        assert_eq!(
            after_reinsert,
            Some(2),
            "re-inserting a bundle must not reset its spray count"
        );
    }
}
