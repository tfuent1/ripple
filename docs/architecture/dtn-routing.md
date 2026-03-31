# DTN Routing

Ripple's routing layer is implemented in `core/src/routing.rs` and `core/src/peer.rs`.
It makes all forwarding decisions based on bundle priority, spray count state, and
peer encounter history. The routing layer never touches a transport or UI directly —
it returns a list of `Action`s for native to execute.

## Design Principles

**Purely functional core.** The `Router` takes inputs and returns outputs. It never
calls back into native platform code. Native calls one of three methods, executes
the returned Actions, and moves on. This keeps the core deterministic and easy to
test without any platform dependencies.

**Store ownership.** `Router` owns the `Store` by value. There is one Router per
process, and it is the single point of access for all routing and persistence
operations. CLI tooling that needs direct store access uses `Router::store()`.

**Spray count in SQLite.** Spray and Wait state is persisted in the `bundles` table
rather than held in memory. This means spray counts survive process restarts, which
matters for a daemon that may be restarted frequently on relay hardware.

## The Three-Method Interface

Native platforms interact with the Router through exactly three methods. All mesh
logic flows through one of these.

**`on_peer_encountered(ed25519_pubkey, x25519_pubkey, transport, rssi, now)`**
Called when a peer is discovered on any transport. Logs the encounter, updates the
in-memory peer table, and returns a `SyncOffer` listing bundle IDs the local node
has queued for this peer. Native uses the offer to coordinate which bundles to
transfer during the sync session.

**`on_bundle_received(bundle, now)`**
Called when a bundle arrives from a peer. Validates the bundle (expiry, hop limit),
increments the hop count, persists it, and returns a list of `Action`s. Returns
`NotifyUser` if the bundle is addressed to this node.

**`on_bundle_forwarded(bundle_id)`**
Called by native after successfully transferring a bundle to a peer. Decrements
the spray count in SQLite. Separated from `on_peer_encountered` because native
controls the actual transfer — the core does not assume a bundle was sent just
because it was offered.

**`mesh_tick(now)`**
Called periodically (~every 30 seconds) by native. Expires bundles past their TTL
and returns any resulting Actions. SOS bundles (`expires_at IS NULL`) are never
expired. Later phases will add rebroadcast scheduling and PRoPHET score decay here.

## Action Enum

Actions are instructions the core returns to native after each method call.
```rust
pub enum Action {
    // Send this bundle to this peer on the next available transport.
    ForwardBundle {
        peer_pubkey: [u8; 32], // X25519 pubkey
        bundle_id:   Uuid,
    },

    // A bundle addressed to this node has arrived — notify the user.
    NotifyUser {
        bundle_id: Uuid,
    },

    // CRDT shared state has been updated — sync native in-memory view.
    // Used when crdt.rs is implemented in Milestone 1.5.
    UpdateSharedState {
        key:   String,
        value: Vec<u8>,
    },
}
```

## Spray and Wait State Machine

Each bundle row in SQLite carries a `spray_remaining` column that drives the
Spray and Wait state machine.
```
spray_remaining > 0  →  Spraying  — forward to every encountered peer, decrement on each transfer
spray_remaining = 0  →  Waiting   — only forward if the encountered peer IS the destination
spray_remaining NULL →  Epidemic  — SOS bundles, forward to every peer always, never transition
```

Initial spray counts are set from `Priority::spray_count()` when a bundle is first
inserted:

| Priority | spray_remaining | Routing |
|---|---|---|
| Normal | 6 | Spray and Wait |
| Urgent | 20 | Spray and Wait |
| SOS | NULL | Epidemic |

When `spray_remaining` reaches 0 the bundle is in the Waiting phase.
`bundles_for_peer` returns it only when the querying peer's X25519 pubkey matches
`dest_pubkey` exactly — meaning we are in direct contact with the destination.

## Peer Tracking

`PeerManager` (in `peer.rs`) maintains an in-memory map of known peers keyed by
Ed25519 pubkey. It is reset on process restart — encounter history for routing
decisions is read from the `encounters` SQLite table, not from the in-memory map.

The two pubkeys on `Peer` serve distinct roles:

- `ed25519_pubkey` — mesh identity, used to verify bundle signatures, map key
- `x25519_pubkey` — encryption key, used to match bundles in `dest_pubkey` column

Never pass an Ed25519 key where an X25519 key is expected or vice versa. See ADR-006.

## Transport Enum

`Transport` formalizes the `u8` codes stored in the `encounters.transport` column:

| Variant | Code | Notes |
|---|---|---|
| `Ble` | 0 | Always on, peer discovery and small bundles |
| `WifiDirect` | 1 | Android bulk sync |
| `Multipeer` | 2 | iOS and macOS bulk sync |
| `WifiAdhoc` | 3 | Desktop and CLI infrastructure nodes |
| `Internet` | 4 | Opportunistic relay, all platforms |
| `Lora` | 5 | Meshtastic bridge, CLI and desktop |

## SyncOffer

`SyncOffer` is returned by `on_peer_encountered`. It contains the bundle IDs the
local node has queued for the encountered peer. Native uses this to negotiate the
transfer — requesting the full bundle bytes for each ID it wants, sending any IDs
the remote peer has that the local node lacks.
```rust
pub struct SyncOffer {
    pub bundle_ids: Vec<Uuid>,
}
```

## Future Considerations

- `mesh_tick` will gain rebroadcast scheduling and PRoPHET encounter score decay
  in Phase 3.
- Interactive routing mode (dense network path-based routing) is a Phase 5
  addition — the current implementation is DTN store-and-forward only.
- The `UpdateSharedState` action is defined but not yet emitted — it becomes
  active when `crdt.rs` is implemented in Milestone 1.5.
```

