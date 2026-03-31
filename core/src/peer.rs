use std::collections::HashMap;

// ── Transport ─────────────────────────────────────────────────────────────────

/// The physical transport a peer was encountered on.
///
/// Stored as a u8 in the `encounters.transport` column.
/// `#[repr(u8)]` tells Rust to use a single byte as the backing integer,
/// so casting `Transport::Ble as u8` gives you `0`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Transport {
    Ble          = 0,
    WifiDirect   = 1,
    Multipeer    = 2,
    WifiAdhoc    = 3,
    Internet     = 4,
    Lora         = 5,
}

impl Transport {
    /// Convert a raw u8 (from SQLite) back into a Transport.
    /// Returns None if the value doesn't match any known variant.
    ///
    /// Rust enums are closed — there's no automatic way to go from an
    /// integer to an enum variant, so we do it explicitly.
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Ble),
            1 => Some(Self::WifiDirect),
            2 => Some(Self::Multipeer),
            3 => Some(Self::WifiAdhoc),
            4 => Some(Self::Internet),
            5 => Some(Self::Lora),
            _ => None,
        }
    }
}

// ── Peer ──────────────────────────────────────────────────────────────────────

/// A known peer node.
///
/// Holds both public keys — Ed25519 for identity/signature verification,
/// X25519 for encryption. These are different keys from the same underlying
/// secret (ADR-006). Don't mix them up at call sites.
#[derive(Debug, Clone)]
pub struct Peer {
    /// Ed25519 public key — this is the peer's mesh identity.
    /// Used to verify bundle signatures. Matches `bundle.origin`.
    pub ed25519_pubkey: [u8; 32],

    /// X25519 public key — used to encrypt direct messages to this peer.
    /// Matches what goes in `Destination::Peer(...)` and `store.dest_pubkey`.
    pub x25519_pubkey: [u8; 32],

    /// When we last saw this peer (Unix timestamp seconds).
    pub last_seen: i64,

    /// Which transport we last saw them on.
    pub transport: Transport,

    /// Signal strength in dBm (negative integer, e.g. -65).
    /// Only meaningful for radio transports (BLE, WiFi). Internet relay
    /// encounters use 0.
    pub rssi: i32,
}

// ── PeerManager ───────────────────────────────────────────────────────────────

/// Tracks all peers encountered during this session.
///
/// Keyed by Ed25519 pubkey — that's the stable identity. X25519 pubkey is
/// derived from the same secret, so it's stable too, but Ed25519 is what
/// bundle signatures use, making it the right key for lookups.
///
/// `HashMap<K, V>` is Rust's standard hash map — equivalent to an associative
/// array in PHP. The key type here is `[u8; 32]` (a fixed-size byte array),
/// which implements `Hash` and `Eq`, so it works as a map key.
pub struct PeerManager {
    peers: HashMap<[u8; 32], Peer>,
}

impl PeerManager {
    pub fn new() -> Self {
        Self {
            peers: HashMap::new(),
        }
    }

    /// Record or update a peer encounter.
    ///
    /// If we've seen this peer before, we update their entry in place.
    /// If not, we insert a new one.
    ///
    /// Returns a shared reference to the updated peer.
    ///
    /// **Rust note:** `.entry(...).or_insert_with(...)` is idiomatic Rust
    /// for "get the value at this key, or insert a default if it's missing,
    /// then give me a mutable reference to whatever's there." It avoids a
    /// double lookup compared to checking `.contains_key()` first.
    pub fn update(
        &mut self,
        ed25519_pubkey: [u8; 32],
        x25519_pubkey: [u8; 32],
        transport: Transport,
        rssi: i32,
        now: i64,
    ) -> &Peer {
        let peer = self.peers.entry(ed25519_pubkey).or_insert_with(|| Peer {
            ed25519_pubkey,
            x25519_pubkey,
            last_seen: now,
            transport,
            rssi,
        });

        // Update mutable fields on existing peers too.
        peer.x25519_pubkey = x25519_pubkey;
        peer.last_seen     = now;
        peer.transport     = transport;
        peer.rssi          = rssi;

        peer
    }

    /// Look up a peer by their Ed25519 pubkey.
    pub fn get(&self, ed25519_pubkey: &[u8; 32]) -> Option<&Peer> {
        self.peers.get(ed25519_pubkey)
    }

    /// All currently known peers.
    pub fn all(&self) -> impl Iterator<Item = &Peer> {
        self.peers.values()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Identity;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn test_transport_roundtrip() {
        // Every defined variant should survive a u8 roundtrip.
        let variants = [
            Transport::Ble,
            Transport::WifiDirect,
            Transport::Multipeer,
            Transport::WifiAdhoc,
            Transport::Internet,
            Transport::Lora,
        ];
        for t in variants {
            let byte = t as u8;
            assert_eq!(Transport::from_u8(byte), Some(t));
        }
        // Unknown value should return None.
        assert_eq!(Transport::from_u8(99), None);
    }

    #[test]
    fn test_peer_manager_insert_and_lookup() {
        let mut manager = PeerManager::new();
        let identity = Identity::generate();

        let ed = identity.public_key();
        let x  = identity.x25519_public_key();

        manager.update(ed, x, Transport::Ble, -65, NOW);

        let peer = manager.get(&ed).unwrap();
        assert_eq!(peer.ed25519_pubkey, ed);
        assert_eq!(peer.x25519_pubkey, x);
        assert_eq!(peer.transport, Transport::Ble);
        assert_eq!(peer.rssi, -65);
    }

    #[test]
    fn test_peer_manager_update_existing() {
        let mut manager = PeerManager::new();
        let identity = Identity::generate();

        let ed = identity.public_key();
        let x  = identity.x25519_public_key();

        manager.update(ed, x, Transport::Ble, -65, NOW);
        // Same peer, seen again on a different transport with better signal.
        manager.update(ed, x, Transport::Internet, -40, NOW + 60);

        let peer = manager.get(&ed).unwrap();
        assert_eq!(peer.transport, Transport::Internet);
        assert_eq!(peer.rssi, -40);
        assert_eq!(peer.last_seen, NOW + 60);

        // Should still be one entry, not two.
        assert_eq!(manager.peers.len(), 1);
    }

    #[test]
    fn test_peer_manager_multiple_peers() {
        let mut manager = PeerManager::new();
        let alice = Identity::generate();
        let bob   = Identity::generate();

        manager.update(alice.public_key(), alice.x25519_public_key(), Transport::Ble, -60, NOW);
        manager.update(bob.public_key(),   bob.x25519_public_key(),   Transport::Ble, -70, NOW);

        assert_eq!(manager.peers.len(), 2);
        assert!(manager.get(&alice.public_key()).is_some());
        assert!(manager.get(&bob.public_key()).is_some());
    }
}
