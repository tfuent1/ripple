# Storage Architecture

Ripple's persistence layer is implemented in `core/src/store.rs` using SQLite
via the `rusqlite` crate with the `bundled` feature — meaning SQLite is compiled
into the binary directly, with no external library dependency.

## Design Principles

**Raw blob + extracted query columns.** Each bundle is stored as a full
MessagePack blob in the `raw` column. Frequently queried fields (`dest_pubkey`,
`expires_at`, `priority`) are extracted into their own columns so the routing
layer can filter without deserializing every bundle. The blob is the source of
truth — the extracted columns are indexes.

**Delivered flag instead of deletion.** Bundles are marked `delivered = 1`
rather than deleted on delivery. This keeps expiry logic simple and leaves the
door open for delivery receipts and retry logic in later phases.

**Nullable `expires_at` for SOS.** SOS priority bundles never expire. Rather
than using a sentinel value (e.g. `i64::MAX`), `expires_at` is nullable —
`NULL` means never expires. The expiry query uses `WHERE expires_at IS NOT NULL`
to naturally exclude SOS bundles.

**`dest_pubkey` is always X25519.** The `dest_pubkey` column stores the
recipient's X25519 public key, not their Ed25519 identity key. These are
different keys — see ADR-006. Passing an Ed25519 key here would silently
store wrong data and cause bundles to never be delivered.

**WAL mode.** The database is opened in Write-Ahead Logging mode for better
concurrent read performance as the CLI and routing layer mature.

## Schema

### `bundles`

| Column | Type | Description |
|---|---|---|
| `id` | TEXT PK | UUID string — bundle identifier |
| `destination` | TEXT | `"peer"` or `"broadcast"` |
| `dest_pubkey` | BLOB | X25519 recipient pubkey (NULL for Broadcast) |
| `priority` | INTEGER | 0 = Normal, 1 = Urgent, 2 = SOS |
| `expires_at` | INTEGER | Unix timestamp seconds, NULL for SOS |
| `delivered` | INTEGER | 0 = pending, 1 = delivered |
| `raw` | BLOB | Full MessagePack-serialized bundle |

Indexes: `dest_pubkey` (partial, non-null only) for `bundles_for_peer` queries;
`expires_at` (partial, non-null only) for `expire_bundles`.

### `encounters`

| Column | Type | Description |
|---|---|---|
| `id` | INTEGER PK | Autoincrement row ID |
| `peer_pubkey` | BLOB | X25519 pubkey of the peer seen |
| `transport` | INTEGER | Transport type code (defined in `peer.rs`) |
| `rssi` | INTEGER | Signal strength in dBm (e.g. -65) |
| `seen_at` | INTEGER | Unix timestamp seconds |

Index: `seen_at` for `recent_encounters` range queries.

## Key Operations

**`insert_bundle`** — serializes the bundle to MessagePack, extracts query
columns, and upserts via `INSERT OR REPLACE`. Idempotent on duplicate IDs.

**`bundles_for_peer(peer_pubkey)`** — returns all undelivered bundles where
`dest_pubkey` matches. Called by the routing layer when a peer is encountered
to determine what to send them.

**`expire_bundles(now)`** — deletes all rows where `expires_at <= now`.
SOS bundles are excluded by the `IS NOT NULL` guard. Returns the count of
deleted bundles. Called by `mesh_tick` in `routing.rs`.

**`all_undelivered`** — returns all bundles with `delivered = 0` regardless
of destination. Used by the CLI relay loop to submit outbound bundles to the
rendezvous server. A Phase 1 simplification — Phase 3 will add transport-aware
filtering so bundles already within BLE range are not redundantly relayed.

**`log_encounter` / `recent_encounters`** — encounter history is the input
to PRoPHET routing (Milestone 1.4+). The `since` parameter on
`recent_encounters` allows the routing layer to look back a configurable
window without scanning the full table.

## Future Considerations

- Delivery receipts and retry tracking may replace the simple `delivered` flag
  in a later phase.
