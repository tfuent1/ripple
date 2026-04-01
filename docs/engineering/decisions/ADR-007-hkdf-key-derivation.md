# ADR-007: HKDF Key Derivation from X25519 Shared Secret

## Status
Accepted

## Context
During the Phase 1 audit, it was identified that `crypto.rs` was passing the
raw X25519 Diffie-Hellman output directly as a ChaCha20-Poly1305 symmetric key:
```rust
let shared_secret = sender_secret.diffie_hellman(&recipient_pub);
let cipher = ChaCha20Poly1305::new_from_slice(shared_secret.as_bytes())?;
```

Raw X25519 output lies on an elliptic curve and has mathematical structure
that violates the uniform-random key assumption required by AEAD cipher suites.
This is a well-documented cryptographic footgun — Signal, Noise Protocol, and
TLS 1.3 all apply a KDF step between DH output and symmetric key derivation
for exactly this reason.

## Decision
Apply HKDF-SHA256 (RFC 5869) to the raw X25519 shared secret before using it
as a ChaCha20-Poly1305 key. The derivation uses a fixed info label to
domain-separate this key from any other potential use of the same shared secret:
```rust
let hk = Hkdf::<Sha256>::new(None, shared_secret.as_bytes());
let mut key_bytes = [0u8; 32];
hk.expand(b"ripple-v1-message", &mut key_bytes)?;
let cipher = ChaCha20Poly1305::new(Key::from_slice(&key_bytes));
```

The `"ripple-v1-message"` info label is intentionally versioned. If the
encryption scheme changes in a future protocol version, the label is bumped
to `"ripple-v2-message"`, ensuring keys derived under different schemes never
collide at any call site.

**Crates added:** `hkdf = "0.12"`, `sha2 = "0.10"`

## Consequences

**Positive:**
- Eliminates the theoretical weakness of using structured curve output as a
  cipher key
- Matches the approach used by Signal, Noise, and TLS 1.3 — well-understood
  and audited pattern
- The versioned info label gives a clean migration path if the scheme changes
- Negligible performance cost — HKDF-SHA256 over 32 bytes is microseconds

**Negative:**
- Wire-breaking change — bundles encrypted before this fix cannot be decrypted
  after it. Since no production traffic existed at the time of the fix (Phase 1
  audit, pre-release), this is acceptable.
- Two additional crate dependencies (`hkdf`, `sha2`)

## Implementation Note
The derivation is isolated in a private `derive_key(shared_secret)` helper
in `crypto.rs`. Both `encrypt()` and `decrypt()` call this helper, ensuring
the key derivation is identical on both sides and cannot diverge.
