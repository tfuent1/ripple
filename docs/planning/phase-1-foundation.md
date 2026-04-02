# Phase 1 — Foundation

## Goal

Implement the complete `ripple-core` library with full test coverage, and a
functional CLI daemon that validates the core works end-to-end. No mobile, no
UI, no BLE. Phase 1 is entirely about getting the protocol right before any
platform complexity is introduced.

A passing Phase 1 means: two CLI nodes running on the same machine (or two
machines on the same network) can discover each other over the internet relay,
exchange bundles, and correctly route, store, forward, expire, and deliver
messages — with all cryptographic guarantees intact.

## Success Criteria

- [x] Ed25519 keypair generation and persistence
- [x] Bundle creation, signing, and signature verification
- [x] Bundle serialization and deserialization (MessagePack)
- [x] Direct message encryption and decryption (X25519 + ChaCha20-Poly1305)
- [x] SQLite store — insert, query, expire, and delete bundles
- [x] Peer encounter logging
- [x] Spray and Wait routing — correct spray count tracking per bundle
- [x] SOS priority epidemic routing
- [x] Bundle TTL expiry via `mesh_tick`
- [x] LWW-CRDT for shared state (map pins, resource posts)
- [x] FFI surface compiles cleanly for both staticlib and cdylib targets
- [x] CLI daemon starts, generates or loads identity, prints public key
- [x] CLI daemon connects to rendezvous server and submits a bundle
- [x] CLI daemon polls rendezvous server inbox and receives a bundle
- [x] Two CLI nodes can exchange a signed, encrypted direct message end-to-end
- [x] Rendezvous server survives restart without losing stored bundles
- [x] Received direct messages display decrypted plaintext in the daemon
- [x] All core modules have unit tests with >80% coverage
- [x] `cargo test` passes clean with no warnings

## Out of Scope

- BLE or WiFi transport (Phase 2)
- Mobile apps (Phase 2)
- Map UI (Phase 2)
- Desktop app (Phase 3)
- Web client (Phase 4)
- PRoPHET routing (Phase 2+)
- Interactive routing mode (Phase 5)
- Mesh namespaces (Phase 3)
- LoRa bridge (Phase 3)

## Milestones

### Milestone 1.1 — Crypto and Identity

Implement `core/src/crypto.rs`. This is the foundation everything else depends on.
No bundle can be created, signed, or encrypted until this module exists.

**Deliverables:**
- `Identity` struct wrapping an Ed25519 keypair
- `Identity::generate()` — creates a new random keypair
- `Identity::from_seed(seed: &[u8])` — deterministic keypair from seed
- `Identity::sign(message: &[u8]) -> [u8; 64]`
- `Identity::verify(message: &[u8], signature: &[u8; 64], pubkey: &[u8; 32]) -> bool`
- `Identity::encrypt(plaintext: &[u8], recipient_pubkey: &[u8; 32]) -> Vec<u8>`
- `Identity::decrypt(ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError>`
- Persistence — serialize/deserialize keypair to/from bytes for secure storage
- Unit tests for all of the above including failure cases

**Crates to add:**
```toml
ed25519-dalek = { version = "2.1", features = ["rand_core"] }
x25519-dalek = { version = "2.0", features = ["static_secrets"] }
chacha20poly1305 = "0.10"
rand = "0.8"
zeroize = { version = "1.7", features = ["derive"] }
```

---

### Milestone 1.2 — Bundle Engine

Implement `core/src/bundle.rs`. Bundles are the atomic unit of communication
in Ripple. Every message, broadcast, and map pin is a bundle.

**Deliverables:**
- `Bundle` struct with all fields from the ADR-004 schema
- `BundleDestination` enum — `Peer([u8; 32])`, `Broadcast`, `ContentHash([u8; 32])`
- `Priority` enum — `Normal(0)`, `Urgent(1)`, `Sos(2)`
- `Bundle::create()` — builds, signs, and optionally encrypts a new bundle
- `Bundle::serialize() -> Vec<u8>` — MessagePack encoding
- `Bundle::deserialize(bytes: &[u8]) -> Result<Bundle, BundleError>`
- `Bundle::verify_signature() -> bool`
- `Bundle::is_expired(now: i64) -> bool`
- `Bundle::decrypt(identity: &Identity) -> Result<Vec<u8>, CryptoError>`
- Unit tests covering serialization round-trips, signature verification,
  expiry logic, and encryption/decryption

---

### Milestone 1.3 — SQLite Store

Implement `core/src/store.rs`. The store is the persistence layer for all
bundles and peer encounter history.

**Deliverables:**
- `Store::new(db_path: &str) -> Result<Store, StoreError>` — opens or creates DB,
  runs migrations
- Schema creation (see `docs/architecture/storage.md`)
- `Store::insert_bundle(bundle: &Bundle) -> Result<(), StoreError>`
- `Store::get_bundle(id: Uuid) -> Result<Option<Bundle>, StoreError>`
- `Store::bundles_for_peer(peer_pubkey: &[u8; 32]) -> Result<Vec<Bundle>, StoreError>`
- `Store::mark_delivered(id: Uuid) -> Result<(), StoreError>`
- `Store::expire_bundles(now: i64) -> Result<u32, StoreError>` — deletes expired,
  returns count
- `Store::log_encounter(peer_pubkey: &[u8; 32], transport: u8, rssi: i32, now: i64)`
- `Store::recent_encounters(since: i64) -> Result<Vec<Encounter>, StoreError>`
- `Store::decrement_spray(id: Uuid) -> Result<Option<u8>, StoreError>`
- Schema versioning via `PRAGMA user_version` — forward-compatible migrations
- Unit tests using an in-memory SQLite DB (`:memory:` path)

---

### Milestone 1.4 — DTN Routing

Implement `core/src/routing.rs` and `core/src/peer.rs`. The routing layer
makes forwarding decisions based on bundle priority and peer encounter history.

**Deliverables (`peer.rs`):**
- `Peer` struct — pubkey, last seen, transport, rssi, spray counts received
- `PeerManager` — tracks known peers, updates on encounter, scores peers for routing

**Deliverables (`routing.rs`):**
- `Router` struct — owns a reference to `Store` and `PeerManager`
- `Router::on_peer_encountered()` — returns `SyncOffer` listing bundle IDs the
  peer might want
- `Router::on_bundle_received()` — stores bundle, returns list of `Action`s
- `Router::on_tick()` — expires bundles, returns list of `Action`s
- `Action` enum — `ForwardBundle { peer, bundle_id }`, `NotifyUser { bundle_id }`,
  `UpdateSharedState { key, value }`
- Spray and Wait — correctly decrements spray count, transitions to Waiting when
  exhausted
- SOS epidemic — never transitions to Waiting, forwards to every peer encountered
- Unit tests covering spray count logic, SOS behavior, expiry actions, and
  duplicate bundle handling

---

### Milestone 1.5 — CRDT Shared State

Implement `core/src/crdt.rs`. Shared state (map pins, resource posts, status
beacons) needs to merge correctly when two nodes sync divergent state without
a central authority.

**Deliverables:**
- `LWWRegister<T>` — Last-Write-Wins register, resolves conflicts by timestamp
  with public key as tiebreaker
- `ORSet<T>` — Observed-Remove Set for additive collections (map pins, contacts)
- `SharedState` — top level struct wrapping a map of `LWWRegister` values and
  `ORSet` collections
- `SharedState::merge(other: &SharedState) -> SharedState` — CRDT merge
- `SharedState::serialize() -> Vec<u8>` — MessagePack encoding
- `SharedState::deserialize(bytes: &[u8]) -> Result<SharedState, CrdtError>`
- Unit tests covering merge commutativity, associativity, and idempotency —
  the three CRDT laws that must hold

---

### Milestone 1.6 — FFI Surface

Implement `ffi/src/lib.rs`. Exposes the core to iOS and Android via a
C-compatible API. Phase 1 only needs enough FFI to verify the surface compiles
correctly — full platform integration is Phase 2.

**Deliverables:**
- `mesh_init(db_path, db_path_len, identity_keypair) -> i32`
- `mesh_peer_encountered(peer_pubkey, transport, rssi, out_sync_offer, out_len) -> i32`
- `mesh_bundle_received(bundle_bytes, bundle_len, from_peer) -> i32`
- `mesh_bundles_for_peer(peer_pubkey, out_bundles, out_len) -> i32`
- `mesh_create_bundle(destination, payload, payload_len, priority, out_bundle, out_len) -> i32`
- `mesh_tick(current_time, out_actions, out_len) -> i32`
- `mesh_free(ptr, len)`
- C header generation via `cbindgen`
- Compiles cleanly as both `staticlib` (iOS) and `cdylib` (Android)

**Crates to add:**
```toml
cbindgen = "0.26"   # in [build-dependencies]
```

---

### Milestone 1.7 — CLI Daemon

Implement `cli/src/main.rs` and supporting modules. The CLI daemon is the
first real end-to-end validation of the entire core.

**Deliverables:**
- `ripple daemon` — starts the mesh daemon, generates or loads identity from
  `~/.ripple/identity`, prints public key on start
- `ripple send <message>` — creates and queues a broadcast bundle
- `ripple send --to <pubkey> <message>` — creates an encrypted direct bundle
- `ripple status` — shows identity, queued bundle count, known peers
- `ripple peers` — lists recently encountered peers
- Internet relay transport — polls rendezvous server for inbox, submits
  outbound bundles
- Periodic `mesh_tick` call every 30 seconds
- Structured logging via `tracing`

**Crates to add:**
```toml
clap = { version = "4.0", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.11", features = ["json"] }
tracing = "0.1"
tracing-subscriber = "0.3"
```

---

### Milestone 1.8 — Rendezvous Server Hardening

Harden the Phase 1 rendezvous stub into a server that survives restarts
and is safe to expose to the network.

**Deliverables:**
- Persistent SQLite DB file (configurable path, default `~/.ripple/rendezvous.db`)
- Bundle size limit — reject bundles over a configurable max (default 64KB)
- Rate limiting — max bundle submissions per source IP per minute
- Replace hand-rolled base64 with the `base64` crate
- `--port` and `--db` CLI flags
- Graceful shutdown — drain in-flight requests before exit
- Docker image with persistent volume mount for the DB file

---

``### Milestone 1.9 — Message Display

Decrypt and display received direct message content in the daemon.

**Deliverables:**
- `bundle.origin_x25519: [u8; 32]` added to `Bundle` — carries the sender's
  X25519 pubkey so recipients can perform correct DH during decryption.
  Ed25519 and X25519 pubkey bytes are not interchangeable (different curve
  encodings) — passing `bundle.origin` to `crypto::decrypt` produces a wrong
  shared secret and silent decryption failure. Discovered during smoke testing.
- `displayed` column added to `bundles` table; `mark_displayed()` and
  `unread_count()` added to `Store`
- On `NotifyUser` — daemon fetches bundle, decrypts payload using
  `crypto::decrypt` with node's own identity and `bundle.origin_x25519`,
  prints sender pubkey prefix and plaintext to stdout, calls `mark_displayed`
- Decryption failure logs a warning and continues — does not crash
- `ripple status` shows unread count (peer bundles delivered but not displayed)
- `--quiet` flag on `ripple daemon` suppresses tracing output; message lines
  and startup pubkey lines always print via `println!`
- `Identity` wrapped in `Arc` in daemon so both async tasks share it without
  borrowing across await points
- Tracing init moved from top of `main()` into the `Daemon` arm only`

---

### Testing Strategy

Phase 1 establishes the testing foundation for the entire project.

**Unit tests** live alongside each module in `core/src/`. Every public function
has at least one happy path and one failure path test. SQLite tests use
`:memory:` to avoid filesystem side effects.

**Integration tests** live in `core/tests/`. Key scenarios:
```
tests/
├── bundle_roundtrip.rs       — create, serialize, deserialize, verify
├── encrypt_decrypt.rs        — encrypt to recipient, decrypt, verify contents
├── spray_and_wait.rs         — spray count tracking, transition to waiting
├── sos_epidemic.rs           — SOS bundles never stop forwarding
├── bundle_expiry.rs          — expired bundles cleaned up on tick
├── crdt_merge.rs             — merge commutativity, associativity, idempotency
└── cli_e2e.rs                — two CLI nodes exchange a message end-to-end
```

**CI** runs on every push and pull request to `main` via GitHub Actions
(`.github/workflows/ci.yml`):
- `cargo fmt --all -- --check` — format gate
- `cargo clippy --all-targets --all-features` — lint gate (`-D warnings` in CI)
- `cargo test --all` — full test suite across all workspace crates
- `cargo deny check` — license compliance, CVE advisory check, banned crate enforcement, duplicate detection (`deny.toml`)
- `cargo geiger --all-features` — unsafe code report (non-blocking)

---

## Definition of Done

Phase 1 is complete when:

1. `cargo test` passes with zero failures and zero warnings (54 tests, 95.4% coverage)
2. `cargo clippy -- -D warnings` passes clean
3. Two `ripple daemon` instances can exchange an encrypted direct message
   through the rendezvous server
4. All success criteria checkboxes above are checked
5. `docs/architecture/storage.md` and `docs/architecture/cryptography.md`
   are written (implementation will surface the details that belong there)
