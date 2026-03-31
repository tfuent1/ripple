use crate::bundle::{Bundle, Destination};
use crate::peer::{PeerManager, Transport};
use crate::store::Store;
use thiserror::Error;
use uuid::Uuid;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum RouterError {
    #[error("store error: {0}")]
    Store(#[from] crate::store::StoreError),

    #[error("bundle error: {0}")]
    Bundle(#[from] crate::bundle::BundleError),
}

// ── Action ────────────────────────────────────────────────────────────────────

/// Instructions returned by the core to the native platform layer.
///
/// The core is purely functional — it never calls back into native code.
/// Instead, it accumulates a list of Actions that native executes after
/// each core call returns.
///
/// In PHP terms, think of this like returning a DTO from a service method
/// that tells the controller what side effects to perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Send this bundle to this peer on the next available transport.
    ForwardBundle {
        /// X25519 pubkey of the peer to forward to.
        /// Native uses this to look up the active transport session.
        peer_pubkey: [u8; 32],
        bundle_id:   Uuid,
    },

    /// A bundle addressed to this node has arrived — notify the user.
    NotifyUser {
        bundle_id: Uuid,
    },

    /// The CRDT shared state has been updated — native should sync its
    /// in-memory view. (Used in Phase 1.5 when crdt.rs is implemented.)
    UpdateSharedState {
        key:   String,
        value: Vec<u8>,
    },
}

// ── SyncOffer ─────────────────────────────────────────────────────────────────

/// Returned when a peer is encountered. Lists bundle IDs the local node
/// has queued for this peer so the two nodes can coordinate what to transfer.
#[derive(Debug, Clone)]
pub struct SyncOffer {
    pub bundle_ids: Vec<Uuid>,
}

// ── Router ────────────────────────────────────────────────────────────────────

/// The routing brain of ripple-core.
///
/// Owns the Store and PeerManager. All routing decisions flow through here.
/// Native platforms call the three public methods and execute the returned
/// Actions — the Router itself never touches a transport or UI.
pub struct Router {
    store:   Store,
    peers:   PeerManager,
    /// Our own X25519 public key — used to detect bundles addressed to us.
    self_x25519_pubkey: [u8; 32],
}

impl Router {
    /// Create a new Router.
    ///
    /// Takes ownership of `store` — once you call this, the caller can no
    /// longer use the Store directly. The Router is the single owner from
    /// this point forward.
    ///
    /// **Rust ownership note:** This is different from passing a reference.
    /// Ownership means the Router is responsible for the Store's lifetime —
    /// when the Router is dropped, the Store (and its SQLite connection) is
    /// dropped too. No manual cleanup needed.
    pub fn new(store: Store, self_x25519_pubkey: [u8; 32]) -> Self {
        Self {
            store,
            peers: PeerManager::new(),
            self_x25519_pubkey,
        }
    }

    /// A peer has been encountered on a transport.
    ///
    /// Records the encounter, updates the peer table, and returns a SyncOffer
    /// listing any bundles we have queued for this peer.
    ///
    /// `peer_ed25519_pubkey` — their identity key (for the peer table)
    /// `peer_x25519_pubkey`  — their encryption key (for bundle matching)
    pub fn on_peer_encountered(
        &mut self,
        peer_ed25519_pubkey: [u8; 32],
        peer_x25519_pubkey:  [u8; 32],
        transport:           Transport,
        rssi:                i32,
        now:                 i64,
    ) -> Result<SyncOffer, RouterError> {
        // Log the encounter in SQLite for PRoPHET scoring later (Phase 3).
        self.store.log_encounter(&peer_x25519_pubkey, transport as u8, rssi, now)?;

        // Update the in-memory peer table.
        self.peers.update(peer_ed25519_pubkey, peer_x25519_pubkey, transport, rssi, now);

        // Find all bundles queued for this specific peer.
        let queued = self.store.bundles_for_peer(&peer_x25519_pubkey)?;

        // For each queued bundle, decide whether to forward based on
        // Spray and Wait state.
        //
        // spray_remaining > 0  → still spraying, forward and decrement
        // spray_remaining = 0  → waiting for direct delivery only
        //                        (but bundles_for_peer already filters by
        //                         dest_pubkey, so any result here IS the
        //                         destination — always forward)
        // spray_remaining NULL → SOS epidemic, always forward
        //
        // In all three cases, if bundles_for_peer returned the bundle,
        // we want to offer it. The spray decrement happens in
        // on_bundle_forwarded (called by native after actual transfer).
        let bundle_ids = queued.iter().map(|b| b.id).collect();

        Ok(SyncOffer { bundle_ids })
    }

    /// Native calls this after successfully transferring a bundle to a peer.
    ///
    /// Decrements the spray count. If spray_remaining hits 0, the bundle
    /// enters the Waiting phase — it will only be forwarded to the
    /// destination peer directly from now on.
    ///
    /// Separated from on_peer_encountered because native controls the
    /// actual transfer — the core shouldn't assume a bundle was sent just
    /// because we offered it.
    pub fn on_bundle_forwarded(&mut self, bundle_id: Uuid) -> Result<(), RouterError> {
        self.store.decrement_spray(bundle_id)?;
        Ok(())
    }

    /// A bundle has arrived from a peer.
    ///
    /// Validates, stores, and returns any Actions the platform should take.
    ///
    /// Returns `NotifyUser` if the bundle is addressed to us.
    /// Returns nothing if the bundle is a broadcast or in-transit relay.
    /// (Broadcast display logic lives in native for Phase 1.)
    pub fn on_bundle_received(
        &mut self,
        bundle: Bundle,
        now:    i64,
    ) -> Result<Vec<Action>, RouterError> {
        // Reject expired bundles immediately — don't store or forward.
        if bundle.is_expired(now) {
            return Ok(vec![]);
        }

        // Reject bundles that have hit the hop limit.
        // We clone here because increment_hop takes &mut self, and we want
        // to check before committing to storage.
        let mut bundle = bundle;
        if !bundle.increment_hop() {
            return Ok(vec![]);
        }

        // Persist. INSERT OR REPLACE means duplicates are silently handled.
        self.store.insert_bundle(&bundle)?;

        let mut actions = vec![];

        match &bundle.destination {
            Destination::Peer(dest_pubkey) => {
                if dest_pubkey == &self.self_x25519_pubkey {
                    // This bundle is for us.
                    actions.push(Action::NotifyUser { bundle_id: bundle.id });
                }
                // If it's not for us, it's stored and will be forwarded
                // when the destination peer is encountered.
            }
            Destination::Broadcast => {
                // Broadcast bundles are stored for forwarding.
                // Display logic (notifying the user of broadcast content)
                // is handled by native in Phase 1.
            }
        }

        Ok(actions)
    }

    /// Periodic heartbeat. Call every ~30 seconds from native.
    ///
    /// Expires old bundles and returns any resulting Actions.
    /// In Phase 1 this is straightforward — later phases will add
    /// rebroadcast scheduling and encounter score decay here.
    pub fn mesh_tick(&mut self, now: i64) -> Result<Vec<Action>, RouterError> {
        // Expire bundles past their TTL. SOS bundles (expires_at IS NULL)
        // are never touched by this query.
        let _expired_count = self.store.expire_bundles(now)?;

        // No actions to return from expiry in Phase 1.
        // Phase 3 will add: rebroadcast scheduling, PRoPHET decay, etc.
        Ok(vec![])
    }

    /// Expose the store for CLI tooling that needs direct access.
    /// Routing logic should always go through Router methods, not this.
    pub fn store(&self) -> &Store {
        &self.store
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::{BundleBuilder, Destination, Priority};
    use crate::crypto::Identity;
    use crate::store::Store;

    const NOW: i64 = 1_700_000_000;

    fn test_router(identity: &Identity) -> Router {
        let store = Store::new(":memory:").unwrap();
        Router::new(store, identity.x25519_public_key())
    }

    // ── on_peer_encountered ───────────────────────────────────────────────────

    #[test]
    fn test_sync_offer_for_queued_bundle() {
        let alice = Identity::generate();
        let bob   = Identity::generate();
        let mut router = test_router(&alice);

        // Queue a bundle for Bob.
        let bundle = BundleBuilder::new(
            Destination::Peer(bob.x25519_public_key()),
            Priority::Normal,
        )
        .payload(b"hey bob".to_vec())
        .build(&alice, NOW)
        .unwrap();

        let bundle_id = bundle.id;
        router.store.insert_bundle(&bundle).unwrap();

        // Bob shows up.
        let offer = router.on_peer_encountered(
            bob.public_key(),
            bob.x25519_public_key(),
            Transport::Ble,
            -65,
            NOW,
        ).unwrap();

        assert_eq!(offer.bundle_ids.len(), 1);
        assert_eq!(offer.bundle_ids[0], bundle_id);
    }

    #[test]
    fn test_sync_offer_empty_for_unknown_peer() {
        let alice = Identity::generate();
        let bob   = Identity::generate();
        let mut router = test_router(&alice);

        // No bundles queued for Bob.
        let offer = router.on_peer_encountered(
            bob.public_key(),
            bob.x25519_public_key(),
            Transport::Ble,
            -65,
            NOW,
        ).unwrap();

        assert!(offer.bundle_ids.is_empty());
    }

    // ── on_bundle_received ────────────────────────────────────────────────────

    #[test]
    fn test_bundle_addressed_to_us_triggers_notify() {
        let alice = Identity::generate();
        let bob   = Identity::generate();
        let mut router = test_router(&bob); // Bob is running this router

        let bundle = BundleBuilder::new(
            Destination::Peer(bob.x25519_public_key()),
            Priority::Normal,
        )
        .payload(b"for bob".to_vec())
        .build(&alice, NOW)
        .unwrap();

        let bundle_id = bundle.id;
        let actions = router.on_bundle_received(bundle, NOW).unwrap();

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0], Action::NotifyUser { bundle_id });
    }

    #[test]
    fn test_bundle_not_for_us_produces_no_notify() {
        let alice   = Identity::generate();
        let bob     = Identity::generate();
        let charlie = Identity::generate();
        let mut router = test_router(&charlie); // Charlie is relaying

        // Bundle is for Bob, not Charlie.
        let bundle = BundleBuilder::new(
            Destination::Peer(bob.x25519_public_key()),
            Priority::Normal,
        )
        .payload(b"for bob".to_vec())
        .build(&alice, NOW)
        .unwrap();

        let actions = router.on_bundle_received(bundle, NOW).unwrap();
        assert!(actions.is_empty());
    }

    #[test]
    fn test_expired_bundle_rejected() {
        let alice = Identity::generate();
        let bob   = Identity::generate();
        let mut router = test_router(&alice);

        let bundle = BundleBuilder::new(
            Destination::Peer(bob.x25519_public_key()),
            Priority::Normal,
        )
        .payload(b"stale".to_vec())
        .build(&alice, NOW)
        .unwrap();

        // Receive it far in the future — past the 24h TTL.
        let actions = router.on_bundle_received(bundle, NOW + 25 * 3600).unwrap();
        assert!(actions.is_empty());
    }

    #[test]
    fn test_broadcast_bundle_stored_no_notify() {
        let alice = Identity::generate();
        let mut router = test_router(&alice);

        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"hello mesh".to_vec())
            .build(&alice, NOW)
            .unwrap();

        let bundle_id = bundle.id;
        let actions = router.on_bundle_received(bundle, NOW).unwrap();

        // No NotifyUser for broadcasts in Phase 1.
        assert!(actions.is_empty());

        // But the bundle should be stored.
        let stored = router.store.get_bundle(bundle_id).unwrap();
        assert!(stored.is_some());
    }

    // ── spray and wait ────────────────────────────────────────────────────────

    #[test]
    fn test_spray_decrement_on_forward() {
        let alice = Identity::generate();
        let bob   = Identity::generate();
        let mut router = test_router(&alice);

        let bundle = BundleBuilder::new(
            Destination::Peer(bob.x25519_public_key()),
            Priority::Normal, // spray_count = 6
        )
        .payload(b"spray".to_vec())
        .build(&alice, NOW)
        .unwrap();

        let bundle_id = bundle.id;
        router.store.insert_bundle(&bundle).unwrap();

        router.on_bundle_forwarded(bundle_id).unwrap();
        router.on_bundle_forwarded(bundle_id).unwrap();

        // Started at 6, decremented twice → 4 remaining.
        let remaining = router.store.decrement_spray(bundle_id).unwrap();
        assert_eq!(remaining, Some(3)); // decrement_spray itself counts as one more
    }

    // ── mesh_tick ─────────────────────────────────────────────────────────────

    #[test]
    fn test_mesh_tick_expires_bundles() {
        let alice = Identity::generate();
        let mut router = test_router(&alice);

        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"temporary".to_vec())
            .build(&alice, NOW)
            .unwrap();

        let bundle_id = bundle.id;
        router.store.insert_bundle(&bundle).unwrap();

        // Tick in the future — past the 24h TTL.
        router.mesh_tick(NOW + 25 * 3600).unwrap();

        // Bundle should be gone.
        assert!(router.store.get_bundle(bundle_id).unwrap().is_none());
    }

    #[test]
    fn test_mesh_tick_preserves_sos() {
        let alice = Identity::generate();
        let mut router = test_router(&alice);

        let sos = BundleBuilder::new(Destination::Broadcast, Priority::Sos)
            .payload(b"mayday".to_vec())
            .build(&alice, NOW)
            .unwrap();

        let sos_id = sos.id;
        router.store.insert_bundle(&sos).unwrap();

        // Tick far into the future.
        router.mesh_tick(NOW + 999_999_999).unwrap();

        // SOS bundle must survive.
        assert!(router.store.get_bundle(sos_id).unwrap().is_some());
    }
}
