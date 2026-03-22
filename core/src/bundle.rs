use crate::crypto::{self, CryptoError, Identity};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum BundleError {
    #[error("serialization failed: {0}")]
    Serialization(#[from] rmp_serde::encode::Error),

    #[error("deserialization failed: {0}")]
    Deserialization(#[from] rmp_serde::decode::Error),

    #[error("crypto error: {0}")]
    Crypto(#[from] CryptoError),

    #[error("bundle signature is invalid")]
    InvalidSignature,

    #[error("bundle has expired")]
    Expired,
}

// ── Priority ──────────────────────────────────────────────────────────────────

/// Routing priority. Drives copy count, TTL, and routing algorithm selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Priority {
    Normal = 0,  // Spray and Wait, N=6,  TTL=24h
    Urgent = 1,  // Spray and Wait, N=20, TTL=12h
    Sos    = 2,  // Epidemic routing,     TTL=never
}

impl Priority {
    /// TTL in seconds. None means the bundle never expires.
    pub fn ttl_seconds(&self) -> Option<i64> {
        match self {
            Priority::Normal => Some(24 * 3600),
            Priority::Urgent => Some(12 * 3600),
            Priority::Sos    => None,
        }
    }

    /// Spray-and-Wait copy count. None means epidemic (unlimited).
    pub fn spray_count(&self) -> Option<u8> {
        match self {
            Priority::Normal => Some(6),
            Priority::Urgent => Some(20),
            Priority::Sos    => None,
        }
    }
}

// ── Destination ───────────────────────────────────────────────────────────────

/// Where a bundle is addressed to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Destination {
    /// Direct message to a specific peer (payload is encrypted).
    Peer([u8; 32]),
    /// Broadcast to all nodes in the namespace (payload is plaintext).
    Broadcast,
    // ContentHash([u8; 32]) — Phase 5, not yet implemented.
}

// ── Bundle ────────────────────────────────────────────────────────────────────

/// The atomic unit of communication in Ripple.
/// Every message, map pin, status beacon, and SOS alert is a Bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bundle {
    pub id:          Uuid,
    pub origin:      [u8; 32],    // Ed25519 pubkey of the sender
    pub destination: Destination,
    pub created_at:  i64,         // Unix timestamp seconds
    pub expires_at:  Option<i64>, // None = never expires (SOS only)
    pub hop_count:   u8,
    pub hop_limit:   u8,
    pub priority:    Priority,
    pub payload:     Vec<u8>,     // encrypted for Peer, plaintext for Broadcast
    pub signature:   Vec<u8>,    // Ed25519 over all fields except signature itself (64 bytes)
}

impl Bundle {
    /// The default hop limit for all bundles.
    pub const DEFAULT_HOP_LIMIT: u8 = 25;

    /// Check whether this bundle has expired relative to a given timestamp.
    pub fn is_expired(&self, now_secs: i64) -> bool {
        match self.expires_at {
            Some(exp) => now_secs >= exp,
            None => false, // SOS bundles never expire
        }
    }

    /// Increment hop count when forwarding. Returns false if hop limit reached.
    pub fn increment_hop(&mut self) -> bool {
        if self.hop_count >= self.hop_limit {
            return false;
        }
        self.hop_count += 1;
        true
    }

    /// Serialize to MessagePack bytes for the wire or SQLite storage.
    pub fn to_bytes(&self) -> Result<Vec<u8>, BundleError> {
        Ok(rmp_serde::to_vec(self)?)
    }

    /// Deserialize from MessagePack bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BundleError> {
        Ok(rmp_serde::from_slice(bytes)?)
    }

    /// Serialize only the fields that are covered by the signature.
    /// This is everything except `signature` itself.
    fn signable_bytes(&self) -> Result<Vec<u8>, BundleError> {
        // We serialize a tuple of the fields — order matters and must never change.
        let signable = (
            &self.id,
            &self.origin,
            &self.destination,
            self.created_at,
            self.expires_at,
            self.hop_count,
            self.hop_limit,
            &self.priority,
            &self.payload,
        );
        Ok(rmp_serde::to_vec(&signable)?)
    }

    /// Verify the bundle's signature against its origin public key.
    pub fn verify(&self) -> Result<(), BundleError> {
        let bytes = self.signable_bytes()?;
        let sig_bytes: [u8; 64] = self.signature.as_slice().try_into()
            .map_err(|_| BundleError::InvalidSignature)?;
        crypto::verify_signature(&self.origin, &bytes, &sig_bytes)
            .map_err(|_| BundleError::InvalidSignature)
    }
}

// ── Builder ───────────────────────────────────────────────────────────────────

/// Fluent builder for creating and signing outbound bundles.
pub struct BundleBuilder {
    destination: Destination,
    priority:    Priority,
    payload:     Vec<u8>,
    hop_limit:   u8,
}

impl BundleBuilder {
    pub fn new(destination: Destination, priority: Priority) -> Self {
        Self {
            destination,
            priority,
            payload: Vec::new(),
            hop_limit: Bundle::DEFAULT_HOP_LIMIT,
        }
    }

    pub fn payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = payload;
        self
    }

    pub fn hop_limit(mut self, limit: u8) -> Self {
        self.hop_limit = limit;
        self
    }

    /// Finalize, sign (and encrypt if addressed to a Peer), returning a Bundle.
    ///
    /// `now_secs` should be the current Unix timestamp. We take it as a
    /// parameter rather than calling the system clock internally — this keeps
    /// the core deterministic and easy to test.
    pub fn build(self, identity: &Identity, now_secs: i64) -> Result<Bundle, BundleError> {
        let expires_at = self.priority.ttl_seconds().map(|ttl| now_secs + ttl);

        // Encrypt payload for direct messages.
        let payload = match &self.destination {
            Destination::Peer(recipient_pubkey) => {
                crypto::encrypt(identity, recipient_pubkey, &self.payload)?
            }
            Destination::Broadcast => self.payload,
        };

        // Build the bundle with a placeholder signature so we can serialize it.
        let mut bundle = Bundle {
            id:          Uuid::new_v4(),
            origin:      identity.public_key(),
            destination: self.destination,
            created_at:  now_secs,
            expires_at,
            hop_count:   0,
            hop_limit:   self.hop_limit,
            priority:    self.priority,
            payload,
            signature:   vec![0u8; 64],
        };

        // Sign the signable fields and write the real signature in.
        let signable = bundle.signable_bytes()?;
        bundle.signature = identity.sign(&signable).to_vec();

        Ok(bundle)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::Identity;

    const NOW: i64 = 1_700_000_000; // fixed timestamp for deterministic tests

    #[test]
    fn test_broadcast_bundle_roundtrip() {
        let identity = Identity::generate();
        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"hello mesh".to_vec())
            .build(&identity, NOW)
            .unwrap();

        // Signature should verify.
        bundle.verify().unwrap();

        // Survives MessagePack roundtrip.
        let bytes = bundle.to_bytes().unwrap();
        let restored = Bundle::from_bytes(&bytes).unwrap();
        assert_eq!(bundle.id, restored.id);
        assert_eq!(restored.payload, b"hello mesh");
        restored.verify().unwrap();
    }

    #[test]
    fn test_direct_message_bundle() {
        let alice = Identity::generate();
        let bob = Identity::generate();

        let bundle = BundleBuilder::new(
            Destination::Peer(bob.x25519_public_key()),
            Priority::Urgent,
        )
        .payload(b"direct message".to_vec())
        .build(&alice, NOW)
        .unwrap();

        bundle.verify().unwrap();

        // Payload should be encrypted — not the original plaintext.
        assert_ne!(bundle.payload, b"direct message");

        // Bob can decrypt it.
        let plaintext = crypto::decrypt(&bob, &alice.x25519_public_key(), &bundle.payload).unwrap();
        assert_eq!(plaintext, b"direct message");
    }

    #[test]
    fn test_sos_never_expires() {
        let identity = Identity::generate();
        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Sos)
            .payload(b"mayday".to_vec())
            .build(&identity, NOW)
            .unwrap();

        assert!(bundle.expires_at.is_none());
        // Should not expire even far in the future.
        assert!(!bundle.is_expired(NOW + 999_999_999));
    }

    #[test]
    fn test_normal_bundle_expires() {
        let identity = Identity::generate();
        let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"temporary".to_vec())
            .build(&identity, NOW)
            .unwrap();

        assert!(!bundle.is_expired(NOW));
        assert!(bundle.is_expired(NOW + 24 * 3600 + 1));
    }

    #[test]
    fn test_tampered_payload_fails_verify() {
        let identity = Identity::generate();
        let mut bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"original".to_vec())
            .build(&identity, NOW)
            .unwrap();

        bundle.payload = b"tampered".to_vec();
        assert!(bundle.verify().is_err());
    }

    #[test]
    fn test_hop_limit_enforced() {
        let identity = Identity::generate();
        let mut bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
            .payload(b"hopping".to_vec())
            .hop_limit(3)
            .build(&identity, NOW)
            .unwrap();

        assert!(bundle.increment_hop()); // 1
        assert!(bundle.increment_hop()); // 2
        assert!(bundle.increment_hop()); // 3 — at limit
        assert!(!bundle.increment_hop()); // refused
    }
}

