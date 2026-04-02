//! Integration tests for the bundle wire format contract.
//!
//! These tests verify that the Bundle struct correctly survives serialization
//! and deserialization (the wire format), that signatures are verified after
//! roundtrips, and that tampering is detected. They test the contract between
//! bundle.rs and the MessagePack encoding — not the internal implementation.
//!
//! Any change to the Bundle struct fields or their serialization order that
//! silently breaks deserialization would be caught here before it reaches
//! a deployed node that can no longer parse bundles from older versions.

use ripple_core::bundle::{Bundle, BundleBuilder, Destination, Priority};
use ripple_core::crypto::Identity;

const NOW: i64 = 1_700_000_000;

// ── Serialization roundtrips ──────────────────────────────────────────────────

#[test]
fn broadcast_bundle_survives_roundtrip() {
    let identity = Identity::generate();
    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"hello mesh".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();

    assert_eq!(bundle.id, restored.id);
    assert_eq!(bundle.origin, restored.origin);
    assert_eq!(bundle.origin_x25519, restored.origin_x25519);
    assert_eq!(bundle.payload, restored.payload);
    assert_eq!(bundle.priority as u8, restored.priority as u8);
    assert_eq!(bundle.expires_at, restored.expires_at);
    assert_eq!(bundle.hop_count, restored.hop_count);
    assert_eq!(bundle.hop_limit, restored.hop_limit);
    assert_eq!(bundle.signature, restored.signature);
}

#[test]
fn direct_message_bundle_survives_roundtrip() {
    let alice = Identity::generate();
    let bob = Identity::generate();

    let bundle = BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Urgent)
        .payload(b"private message".to_vec())
        .build(&alice, NOW)
        .unwrap();

    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();

    assert_eq!(bundle.id, restored.id);
    // Destination must survive exactly — wrong pubkey would cause misrouting.
    assert_eq!(bundle.destination, restored.destination);
    // Payload is encrypted — must be byte-for-byte identical after roundtrip.
    assert_eq!(bundle.payload, restored.payload);
}

#[test]
fn sos_bundle_survives_roundtrip() {
    let identity = Identity::generate();
    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Sos)
        .payload(b"mayday".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();

    // SOS bundles never expire — this must survive the roundtrip as None.
    assert!(restored.expires_at.is_none());
    assert!(!restored.is_expired(NOW + 999_999_999));
}

#[test]
fn all_three_priorities_roundtrip_correctly() {
    let identity = Identity::generate();

    for priority in [Priority::Normal, Priority::Urgent, Priority::Sos] {
        let bundle = BundleBuilder::new(Destination::Broadcast, priority)
            .payload(b"test".to_vec())
            .build(&identity, NOW)
            .unwrap();

        let bytes = bundle.to_bytes().unwrap();
        let restored = Bundle::from_bytes(&bytes).unwrap();

        // Priority as u8 comparison since Priority doesn't derive PartialEq
        // across the public API boundary.
        assert_eq!(bundle.priority as u8, restored.priority as u8);
    }
}

// ── Signature verification across roundtrip ───────────────────────────────────

#[test]
fn signature_verifies_after_roundtrip() {
    let identity = Identity::generate();
    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"signed payload".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();

    // Signature must still be valid — if serialization corrupts any signed
    // field, this catches it.
    restored.verify().unwrap();
}

#[test]
fn signature_verifies_after_hop_count_increment() {
    // hop_count is intentionally excluded from the signature (it's mutated
    // in transit). Verify that incrementing it doesn't break verification.
    let identity = Identity::generate();
    let mut bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"relay me".to_vec())
        .build(&identity, NOW)
        .unwrap();

    // Simulate three relay hops.
    assert!(bundle.increment_hop());
    assert!(bundle.increment_hop());
    assert!(bundle.increment_hop());

    // Roundtrip after mutation.
    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();

    assert_eq!(restored.hop_count, 3);
    restored.verify().unwrap(); // Must still pass.
}

// ── Tampering detection ───────────────────────────────────────────────────────

#[test]
fn tampered_payload_fails_verification() {
    let identity = Identity::generate();
    let mut bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"original".to_vec())
        .build(&identity, NOW)
        .unwrap();

    bundle.payload = b"tampered".to_vec();
    assert!(bundle.verify().is_err());

    // Also fails after roundtrip — can't serialize your way out of it.
    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();
    assert!(restored.verify().is_err());
}

#[test]
fn tampered_origin_fails_verification() {
    let identity = Identity::generate();
    let impostor = Identity::generate();
    let mut bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"from alice".to_vec())
        .build(&identity, NOW)
        .unwrap();

    // Replace origin with a different public key — signature no longer matches.
    bundle.origin = impostor.public_key();
    assert!(bundle.verify().is_err());
}

#[test]
fn tampered_destination_fails_verification() {
    let alice = Identity::generate();
    let bob = Identity::generate();
    let eve = Identity::generate();

    let mut bundle =
        BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
            .payload(b"for bob".to_vec())
            .build(&alice, NOW)
            .unwrap();

    // Redirect to Eve's pubkey — must fail signature check.
    bundle.destination = Destination::Peer(eve.x25519_public_key());
    assert!(bundle.verify().is_err());
}

#[test]
fn corrupted_signature_bytes_fail_verification() {
    let identity = Identity::generate();
    let mut bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"clean".to_vec())
        .build(&identity, NOW)
        .unwrap();

    // Zero out the signature — clearly invalid.
    bundle.signature = vec![0u8; 64];
    assert!(bundle.verify().is_err());
}

// ── Expiry logic ──────────────────────────────────────────────────────────────

#[test]
fn normal_bundle_expires_at_correct_timestamp() {
    let identity = Identity::generate();
    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"expires".to_vec())
        .build(&identity, NOW)
        .unwrap();

    assert!(!bundle.is_expired(NOW));
    assert!(!bundle.is_expired(NOW + 24 * 3600 - 1));
    assert!(bundle.is_expired(NOW + 24 * 3600));
}

#[test]
fn urgent_bundle_expires_at_correct_timestamp() {
    let identity = Identity::generate();
    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Urgent)
        .payload(b"urgent".to_vec())
        .build(&identity, NOW)
        .unwrap();

    assert!(!bundle.is_expired(NOW + 12 * 3600 - 1));
    assert!(bundle.is_expired(NOW + 12 * 3600));
}

// ── Hop limit ─────────────────────────────────────────────────────────────────

#[test]
fn hop_limit_enforced_at_boundary() {
    let identity = Identity::generate();
    let mut bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"hops".to_vec())
        .hop_limit(3)
        .build(&identity, NOW)
        .unwrap();

    assert_eq!(bundle.hop_count, 0);
    assert!(bundle.increment_hop()); // → 1
    assert!(bundle.increment_hop()); // → 2
    assert!(bundle.increment_hop()); // → 3, at limit
    assert!(!bundle.increment_hop()); // refused — already at limit
    assert_eq!(bundle.hop_count, 3);
}

// ── Edge cases ────────────────────────────────────────────────────────────────

#[test]
fn empty_payload_roundtrips() {
    let identity = Identity::generate();
    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(vec![])
        .build(&identity, NOW)
        .unwrap();

    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();
    assert!(restored.payload.is_empty());
    restored.verify().unwrap();
}

#[test]
fn max_payload_roundtrips() {
    let identity = Identity::generate();
    let large_payload = vec![0u8; 64 * 1024]; // 64KB, rendezvous server limit
    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(large_payload.clone())
        .build(&identity, NOW)
        .unwrap();

    let bytes = bundle.to_bytes().unwrap();
    let restored = Bundle::from_bytes(&bytes).unwrap();
    assert_eq!(restored.payload, large_payload);
    restored.verify().unwrap();
}

#[test]
fn deserializing_garbage_bytes_returns_error() {
    let result = Bundle::from_bytes(b"this is not a bundle");
    assert!(result.is_err());
}

#[test]
fn deserializing_empty_bytes_returns_error() {
    let result = Bundle::from_bytes(&[]);
    assert!(result.is_err());
}
