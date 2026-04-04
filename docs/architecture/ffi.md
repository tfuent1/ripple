# FFI Surface

Ripple's C FFI surface is implemented in `ffi/src/lib.rs`. It exposes
`ripple-core` to iOS (via Swift/XCFramework) and Android (via Kotlin/JNI)
through a C-compatible API. The CLI and desktop targets import `ripple-core`
directly as a Rust crate and do not use this surface.

## Design Principles

**Purely functional boundary.** Every FFI function takes inputs, does work
through the core, and returns outputs. No callbacks cross the boundary in
either direction. Native calls a function, gets a result, executes any
returned Actions, and moves on.

**Process-global singletons.** Two singletons are initialized by `mesh_init`:
`IDENTITY` (holds the node's `Identity` — Ed25519 signing key and derived
X25519 keypair) and `ROUTER` (holds the `Router`, which owns Store and
PeerManager). Lock ordering rule: always acquire `IDENTITY` before `ROUTER`
if both are needed in the same call path. In practice no current function
needs both. There is no context pointer — the C side is stateless.

**MessagePack over the boundary.** All structured outputs are serialized to
MessagePack before being written to the caller's out-pointer. The caller
receives opaque bytes and deserializes on their side. This keeps the FFI
surface minimal and avoids complex type mapping between Rust and Swift/Kotlin.

**Allocate-and-return memory model.** Output buffers are allocated by Rust
on the heap and ownership is transferred to the caller. The caller must call
`mesh_free(ptr, len)` exactly once when done. Failing to do so leaks memory.
Calling it twice is undefined behavior.

## Return Codes

Every function returns an `i32` status code.

| Code | Meaning |
|---|---|
| `0` | Success |
| `-1` | Not initialized — call `mesh_init` first |
| `-2` | Serialization error |
| `-3` | Internal error (store, routing, crypto) |

## Function Reference

### `mesh_init`
```c
int32_t mesh_init(
    const uint8_t *db_path,
    uintptr_t      db_path_len,
    const uint8_t *identity_bytes,
    uintptr_t      identity_len
);
```

Opens or creates the SQLite database at `db_path` (UTF-8, not
null-terminated — length given by `db_path_len`). Loads the identity from
`identity_bytes` (32-byte Ed25519 private key). If `identity_bytes` is all
zeros or `identity_len` is not 32, a new identity is generated.

Initializes the global Router singleton. Returns `-1` if called a second
time — `OnceLock` sets exactly once.

Must be called before any other function.

---

### `mesh_peer_encountered`
```c
int32_t mesh_peer_encountered(
    const uint8_t *ed25519_pubkey,
    const uint8_t *x25519_pubkey,
    uint8_t        transport,
    int32_t        rssi,
    int64_t        now,
    uint8_t      **out_offer,
    uintptr_t     *out_offer_len
);
```

Notifies the core that a peer has been encountered. Both pubkeys are 32
bytes. `transport` is a Transport enum code (see table below). `rssi` is
signal strength in dBm — pass `0` for internet transport encounters.

Writes a MessagePack-serialized `Vec<[u8; 16]>` (bundle IDs as raw UUID
bytes) to `out_offer`. Caller must `mesh_free(out_offer, out_offer_len)`
when done.

---

### `mesh_bundle_received`
```c
int32_t mesh_bundle_received(
    const uint8_t *bundle_bytes,
    uintptr_t      bundle_len,
    int64_t        now,
    uint8_t      **out_actions,
    uintptr_t     *out_actions_len
);
```

Hands a received bundle (raw MessagePack bytes) to the core. The core
validates, stores, and returns any Actions.

Writes a MessagePack-serialized `Vec<SerializableAction>` to `out_actions`.
Caller must `mesh_free` when done.

---

### `mesh_bundle_forwarded`
```c
int32_t mesh_bundle_forwarded(
    const uint8_t *bundle_id_bytes,
    uintptr_t      bundle_id_len
);
```

Notifies the core that a bundle was successfully transferred to a peer.
Decrements the spray count for Spray and Wait routing. `bundle_id_bytes`
must be exactly 16 bytes (raw UUID). No output buffer — returns a status
code only.

---

### `mesh_bundles_for_peer`
```c
int32_t mesh_bundles_for_peer(
    const uint8_t *x25519_pubkey,
    uint8_t      **out_bundles,
    uintptr_t     *out_bundles_len
);
```

Returns all undelivered bundles queued for a peer identified by their
X25519 pubkey (32 bytes).

Writes a MessagePack-serialized `Vec<Vec<u8>>` to `out_bundles` — each
inner `Vec<u8>` is a complete MessagePack-serialized Bundle, ready to
send over the transport. Caller must `mesh_free` when done.

---

### `mesh_create_bundle`
```c
int32_t mesh_create_bundle(
    const uint8_t *dest_pubkey,
    const uint8_t *payload,
    uintptr_t      payload_len,
    uint8_t        priority,
    int64_t        now,
    uint8_t      **out_bundle,
    uintptr_t     *out_bundle_len
);
```

Creates, signs, stores, and returns a new outbound bundle. Signs using the
identity loaded at `mesh_init` — the private key is never passed over the
FFI boundary after initialization. See ADR-008 (Option C).

`dest_pubkey` — 32-byte X25519 pubkey of the recipient for a direct
message, or NULL for a broadcast bundle. **Must be X25519, not Ed25519.**
See ADR-006.

`priority` — `0` = Normal, `1` = Urgent, `2` = SOS.

Writes a MessagePack-serialized Bundle to `out_bundle`. Caller must
`mesh_free` when done.

---

### `mesh_tick`
```c
int32_t mesh_tick(
    int64_t    now,
    uint8_t  **out_actions,
    uintptr_t *out_actions_len
);
```

Periodic heartbeat. Call every ~30 seconds from native. Expires bundles
past their TTL and returns any resulting Actions. SOS bundles never expire.

Writes a MessagePack-serialized `Vec<SerializableAction>` to `out_actions`.
Caller must `mesh_free` when done.

---

### `mesh_free`
```c
void mesh_free(uint8_t *ptr, uintptr_t len);
```

Frees a buffer previously allocated by this library. Must be called exactly
once for every out-pointer written by any other function. Passing a pointer
not allocated here, or calling this twice on the same pointer, is undefined
behavior. Passing NULL is safe — the function returns immediately.

---

## Transport Codes

The `transport` parameter on `mesh_peer_encountered` uses the same integer
codes as the `encounters.transport` SQLite column.

| Code | Variant | Notes |
|---|---|---|
| `0` | BLE | Always on, peer discovery and small bundles |
| `1` | WiFi Direct | Android bulk sync |
| `2` | Multipeer | iOS and macOS bulk sync |
| `3` | WiFi Ad-hoc | Desktop and CLI infrastructure nodes |
| `4` | Internet | Opportunistic relay, all platforms |
| `5` | LoRa | Meshtastic bridge, CLI and desktop |

## SerializableAction Schema

Actions returned by `mesh_bundle_received` and `mesh_tick` are serialized
as a MessagePack array of tagged maps. Each map has a `type` field
identifying the variant.

**`ForwardBundle`**
```json
{ "type": "ForwardBundle", "peer_pubkey": <bytes>, "bundle_id": <16 bytes> }
```
Send the identified bundle to the identified peer on the next available
transport. `peer_pubkey` is the peer's X25519 pubkey (32 bytes).

**`NotifyUser`**
```json
{ "type": "NotifyUser", "bundle_id": <16 bytes> }
```
A bundle addressed to this node has arrived. Display it to the user.

**`UpdateSharedState`**
```json
{ "type": "UpdateSharedState", "key": <string>, "value": <bytes> }
```
CRDT shared state has been updated. Sync the native in-memory view.
Defined but not yet emitted in Phase 1 — wired in a later milestone.

## UUID Representation

Bundle IDs cross the FFI boundary as raw 16-byte arrays (`[u8; 16]`),
not as strings. This is more compact and avoids encoding ambiguity.
Native platforms should use their UUID type's byte constructor when
reading these values.

## Memory Contract Summary
```
Rust allocates → writes ptr+len to out-params → returns OK
Caller uses buffer
Caller calls mesh_free(ptr, len)
Rust reconstructs Box, drops it, memory freed
```

Every function that writes to an out-pointer follows this contract without
exception. The caller must track which out-pointers have been written and
free them all, including on error paths where partial output may have been
written before the error was detected.

## Future Considerations

- `cbindgen` header generation deferred to Phase 2. For now, callers
  maintain their own header or use the function signatures above directly.
- `mesh_create_bundle` uses the `IDENTITY` singleton initialized at
  `mesh_init`. The private key is never passed over the FFI boundary
  after startup. See ADR-008 (Option C, adopted Phase 2).
- A `mesh_decrypt_bundle` function will be needed in Phase 2 when the
  native layer needs to display received direct messages to the user.
  The CLI handles this directly via `crypto::decrypt` without going through
  FFI — this gap only applies to iOS and Android.
- `mesh_init` null pointer guard on `identity_bytes` added (Phase 1 hardening)
- `mesh_create_bundle` private key wrapped in `Zeroizing<[u8; 32]>` to guarantee zeroing on drop (Phase 1 hardening)
