//! Integration tests for the encryption/decryption contract.
//!
//! These tests exercise `crypto::encrypt` and `crypto::decrypt` across
//! the full public API surface — including the two-key identity model
//! (ADR-006) and the HKDF key derivation step (ADR-007).
//!
//! The unit tests in `crypto.rs` verify the functions work in isolation.
//! These tests verify the contract holds end-to-end: that the output of
//! `encrypt` is exactly what `decrypt` needs, and that variations in key
//! material produce the right failures.

use ripple_core::bundle::{BundleBuilder, Destination, Priority};
use ripple_core::crypto::{self, Identity};

const NOW: i64 = 1_700_000_000;

// ── Basic encrypt/decrypt contract ────────────────────────────────────────────

#[test]
fn alice_encrypts_bob_decrypts() {
    let alice = Identity::generate();
    let bob = Identity::generate();

    let plaintext = b"hello from alice";
    let ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), plaintext).unwrap();

    // Ciphertext must not be the plaintext.
    assert_ne!(ciphertext, plaintext);

    let recovered = crypto::decrypt(&bob, &alice.x25519_public_key(), &ciphertext).unwrap();
    assert_eq!(recovered, plaintext);
}

#[test]
fn encrypt_produces_different_ciphertext_each_call() {
    // Each call generates a fresh random nonce — same plaintext must produce
    // different ciphertext. If this fails it means nonces are not random,
    // which would be a catastrophic security failure.
    let alice = Identity::generate();
    let bob = Identity::generate();

    let ct1 = crypto::encrypt(&alice, &bob.x25519_public_key(), b"same message").unwrap();
    let ct2 = crypto::encrypt(&alice, &bob.x25519_public_key(), b"same message").unwrap();

    assert_ne!(ct1, ct2, "nonces must be unique per encryption");
}

#[test]
fn empty_plaintext_encrypts_and_decrypts() {
    let alice = Identity::generate();
    let bob = Identity::generate();

    let ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), b"").unwrap();
    let recovered = crypto::decrypt(&bob, &alice.x25519_public_key(), &ciphertext).unwrap();
    assert_eq!(recovered, b"");
}

#[test]
fn large_payload_encrypts_and_decrypts() {
    let alice = Identity::generate();
    let bob = Identity::generate();

    let plaintext = vec![0xABu8; 64 * 1024]; // 64KB
    let ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), &plaintext).unwrap();
    let recovered = crypto::decrypt(&bob, &alice.x25519_public_key(), &ciphertext).unwrap();
    assert_eq!(recovered, plaintext);
}

// ── Eve cannot decrypt ────────────────────────────────────────────────────────

#[test]
fn eve_cannot_decrypt_message_intended_for_bob() {
    let alice = Identity::generate();
    let bob = Identity::generate();
    let eve = Identity::generate();

    let ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), b"for bob only").unwrap();

    // Eve tries to decrypt using her own private key — must fail.
    let result = crypto::decrypt(&eve, &alice.x25519_public_key(), &ciphertext);
    assert!(
        result.is_err(),
        "eve must not be able to decrypt a message intended for bob"
    );
}

#[test]
fn wrong_sender_key_fails_decryption() {
    // Bob correctly has the ciphertext but is told it came from Eve, not Alice.
    // The DH shared secret will be wrong — decryption must fail.
    let alice = Identity::generate();
    let bob = Identity::generate();
    let eve = Identity::generate();

    let ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), b"from alice").unwrap();

    // Bob tries to decrypt claiming sender is Eve.
    let result = crypto::decrypt(&bob, &eve.x25519_public_key(), &ciphertext);
    assert!(
        result.is_err(),
        "decryption with wrong sender key must fail"
    );
}

// ── ADR-006: two-key model — Ed25519 ≠ X25519 ────────────────────────────────

#[test]
fn using_ed25519_key_where_x25519_expected_fails_decryption() {
    // ADR-006 critical invariant: `bundle.origin` (Ed25519) and
    // `bundle.origin_x25519` (X25519) are not interchangeable even though
    // both are [u8; 32]. Passing the Ed25519 pubkey to decrypt() where the
    // X25519 pubkey is required produces a wrong shared secret.
    //
    // This test documents that the failure is a decryption error, not a
    // panic or silent wrong answer — callers get a clear Result::Err.
    let alice = Identity::generate();
    let bob = Identity::generate();

    let ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), b"secret").unwrap();

    // Pass Alice's Ed25519 pubkey where her X25519 pubkey is required.
    // These are different curve encodings of the same scalar — this is the
    // exact mistake ADR-006 warns against at every call site.
    let ed25519_pubkey = alice.public_key(); // Ed25519 — wrong key type
    let result = crypto::decrypt(&bob, &ed25519_pubkey, &ciphertext);

    assert!(
        result.is_err(),
        "using Ed25519 key where X25519 is required must fail, not silently decrypt garbage"
    );
}

// ── Ciphertext integrity ──────────────────────────────────────────────────────

#[test]
fn truncated_ciphertext_fails_decryption() {
    let alice = Identity::generate();
    let bob = Identity::generate();

    let ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), b"message").unwrap();

    // Truncate to fewer than the 12-byte nonce — must return CiphertextTooShort.
    let truncated = &ciphertext[..6];
    let result = crypto::decrypt(&bob, &alice.x25519_public_key(), truncated);
    assert!(result.is_err());
}

#[test]
fn flipped_bit_in_ciphertext_fails_decryption() {
    // ChaCha20-Poly1305 is an AEAD — any modification to the ciphertext
    // (or the authentication tag) must be detected and rejected.
    let alice = Identity::generate();
    let bob = Identity::generate();

    let mut ciphertext = crypto::encrypt(&alice, &bob.x25519_public_key(), b"authentic").unwrap();

    // Flip a bit in the ciphertext body (past the 12-byte nonce).
    let body_idx = 12;
    ciphertext[body_idx] ^= 0xFF;

    let result = crypto::decrypt(&bob, &alice.x25519_public_key(), &ciphertext);
    assert!(result.is_err(), "AEAD must reject tampered ciphertext");
}

// ── End-to-end: bundle encryption matches crypto layer ────────────────────────

#[test]
fn bundle_encrypt_decrypt_matches_crypto_layer() {
    // The bundle layer calls crypto::encrypt internally during build().
    // This test verifies that the payload the bundle stores is exactly
    // what crypto::decrypt expects — i.e., the bundle doesn't add any
    // framing or transformation on top of the raw ciphertext.
    let alice = Identity::generate();
    let bob = Identity::generate();

    let plaintext = b"end to end";

    let bundle = BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
        .payload(plaintext.to_vec())
        .build(&alice, NOW)
        .unwrap();

    // bundle.payload is the raw AEAD output — decrypt it directly.
    let recovered = crypto::decrypt(&bob, &alice.x25519_public_key(), &bundle.payload).unwrap();

    assert_eq!(recovered, plaintext);
}

#[test]
fn bundle_origin_x25519_is_correct_sender_key_for_decryption() {
    // This test verifies the specific field the daemon uses: bundle.origin_x25519.
    // It must equal alice.x25519_public_key() — not alice.public_key() (Ed25519).
    // The bug this guards against was discovered in Milestone 1.9 smoke testing.
    let alice = Identity::generate();
    let bob = Identity::generate();

    let bundle = BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
        .payload(b"verify the field".to_vec())
        .build(&alice, NOW)
        .unwrap();

    // This is exactly what the daemon does on NotifyUser.
    let recovered = crypto::decrypt(&bob, &bundle.origin_x25519, &bundle.payload).unwrap();

    assert_eq!(recovered, b"verify the field");

    // Confirm origin_x25519 matches alice's X25519 key, not her Ed25519 key.
    assert_eq!(bundle.origin_x25519, alice.x25519_public_key());
    assert_ne!(bundle.origin_x25519, alice.public_key()); // Ed25519 ≠ X25519
}
