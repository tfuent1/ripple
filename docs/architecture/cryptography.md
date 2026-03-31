# Cryptography

Ripple's cryptographic layer is implemented in `core/src/crypto.rs` using
pure Rust crates from the RustCrypto project. No C dependencies — see ADR-005.

## Design Principles

**Two-key identity model.** Every node has an Ed25519 signing keypair and a
derived X25519 encryption keypair. They share the same private scalar but serve
distinct roles and must not be confused at call sites. See ADR-006.

**One secret persisted.** Only the Ed25519 private key is stored (32 bytes).
The X25519 secret is re-derived from it at runtime via `StaticSecret::from(signing_key.to_bytes())`.

**ZeroizeOnDrop.** The `Identity` struct is annotated with `#[derive(ZeroizeOnDrop)]`.
When an `Identity` is dropped, the private key bytes are overwritten in memory
before deallocation. This prevents key material from lingering in process memory
after the identity is no longer needed.

**Nonce-per-message.** ChaCha20-Poly1305 encryption generates a fresh random
12-byte nonce for every message. The nonce is prepended to the ciphertext and
sent with it — the recipient splits them apart before decrypting.

## Identity
```rust
pub struct Identity {
    signing_key: SigningKey, // ed25519-dalek
}
```

| Method | Description |
|---|---|
| `Identity::generate()` | Generate a new random keypair |
| `Identity::from_bytes(bytes: &[u8; 32])` | Load from stored private key bytes |
| `Identity::to_private_bytes() -> [u8; 32]` | Export for secure storage |
| `Identity::public_key() -> [u8; 32]` | Ed25519 pubkey — mesh identity, used to verify signatures |
| `Identity::x25519_public_key() -> [u8; 32]` | X25519 pubkey — used to encrypt messages to this node |
| `Identity::sign(message: &[u8]) -> [u8; 64]` | Sign arbitrary bytes |

## Key Roles

| Key | Type | Advertised | Used for |
|---|---|---|---|
| `public_key()` | Ed25519 | Yes | Bundle signature verification, mesh identity, `bundle.origin` |
| `x25519_public_key()` | X25519 | Yes | Encrypting direct messages, `Destination::Peer(...)`, `dest_pubkey` column |

**Never pass an Ed25519 key where an X25519 key is expected, or vice versa.**
The types are both `[u8; 32]` — the compiler cannot catch this mistake. See ADR-006.

## Signing and Verification

All bundles are signed by their origin node using Ed25519. The signature covers
every bundle field except the `signature` field itself, serialized as MessagePack.
```rust
// Signing (in BundleBuilder::build)
let signable = bundle.signable_bytes()?;
bundle.signature = identity.sign(&signable).to_vec();

// Verification (in Bundle::verify)
crypto::verify_signature(&self.origin, &bytes, &sig_bytes)
```

Verification takes the raw Ed25519 pubkey from `bundle.origin` — not the X25519
key. The origin field is always Ed25519.

## Encryption

Direct messages use X25519 Diffie-Hellman key exchange followed by
ChaCha20-Poly1305 authenticated encryption.

**Encrypt (sender side):**
1. Derive shared secret: sender X25519 secret × recipient X25519 public key
2. Use shared secret as ChaCha20-Poly1305 symmetric key
3. Generate random 12-byte nonce
4. Encrypt plaintext → ciphertext + 16-byte authentication tag
5. Output: `nonce (12 bytes) || ciphertext+tag`

**Decrypt (recipient side):**
1. Split input into nonce (first 12 bytes) and ciphertext
2. Derive shared secret: recipient X25519 secret × sender X25519 public key
3. Decrypt and verify authentication tag

The shared secret is symmetric — both sides independently compute the same value
from their own private key and the other party's public key. No key exchange
round-trip is required.

## Crates

| Crate | Version | Role |
|---|---|---|
| `ed25519-dalek` | 2.1 | Ed25519 signing and verification |
| `x25519-dalek` | 2.0 | X25519 Diffie-Hellman key exchange |
| `chacha20poly1305` | 0.10 | Authenticated symmetric encryption |
| `zeroize` | 1.7 | Secure memory zeroing on drop |
| `rand` | 0.8 | Nonce and keypair generation |

## Error Types
```rust
pub enum CryptoError {
    VerificationFailed,   // signature check failed
    EncryptionFailed,     // AEAD encryption error
    DecryptionFailed,     // AEAD decryption or auth tag failure
    InvalidPublicKey,     // malformed Ed25519 pubkey bytes
    CiphertextTooShort,   // input shorter than nonce length (12 bytes)
}
```

## Future Considerations

- Bundle payload encryption currently uses raw X25519 DH output as the
  ChaCha20-Poly1305 key. A KDF (HKDF) over the shared secret would be
  more robust and is a candidate hardening step before Phase 2.
- Namespace shared-key encryption (Phase 3) will add a separate symmetric
  encryption path for broadcast bundles within private namespaces.
