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
| `x25519_public_key()` | X25519 | Yes | Encrypting direct messages, `Destination::Peer(...)`, `dest_pubkey` column, `bundle.origin_x25519` |

**Never pass an Ed25519 key where an X25519 key is expected, or vice versa.**
The types are both `[u8; 32]` — the compiler cannot catch this mistake. See ADR-006.

**`bundle.origin` and `bundle.origin_x25519` are not interchangeable.**
Both are `[u8; 32]` and both belong to the sender, but they are different encodings
of the same underlying Curve25519 point (Edwards form vs Montgomery form). `origin`
is used for signature verification. `origin_x25519` is used as the sender's DH
public key during decryption. Passing `origin` where `origin_x25519` is expected
produces a wrong shared secret and silent decryption failure — no compile error.
This is the bug that was discovered during Milestone 1.9 smoke testing.

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
2. Derive symmetric key: HKDF-SHA256(shared_secret, info="ripple-v1-message")
3. Generate random 12-byte nonce
4. Encrypt plaintext → ciphertext + 16-byte authentication tag
5. Output: `nonce (12 bytes) || ciphertext+tag`

**Decrypt (recipient side):**
1. Split input into nonce (first 12 bytes) and ciphertext
2. Derive shared secret: recipient X25519 secret × sender X25519 public key
   (taken from `bundle.origin_x25519` — NOT `bundle.origin`)
3. Derive symmetric key: HKDF-SHA256(shared_secret, info="ripple-v1-message")
4. Decrypt and verify authentication tag

The shared secret is symmetric — both sides independently compute the same value
from their own private key and the other party's public key. No key exchange
round-trip is required.

## Nonce Security

Each ChaCha20-Poly1305 encryption call generates a fresh 12-byte nonce via
`ChaCha20Poly1305::generate_nonce(&mut OsRng)`. The nonce is prepended to the
ciphertext and transmitted with it.

**Nonce reuse threat.** If the same nonce were used twice with the same key,
an attacker who observes both ciphertexts can XOR them to cancel the keystream
and recover information about both plaintexts — a catastrophic failure mode for
any stream cipher. ChaCha20-Poly1305 provides no protection against this.

**Why random nonces are safe here.** The 12-byte (96-bit) nonce space gives
2^96 ≈ 79 octillion possible values. For a collision to occur by chance, a
single sender-recipient pair would need to exchange roughly 2^48 messages
(birthday bound). At one message per second that is ~9 million years. Random
nonce generation via the OS CSPRNG (`OsRng`) is the correct approach for this
message volume.

**What this does not protect against.** A broken or compromised OS random
number generator could produce repeated nonces. This is a systemic risk
affecting all software on the device and is outside Ripple's threat model.
Platform-level CSPRNG integrity is assumed.

**Key rotation.** Each message uses a freshly derived ECDH shared secret
because the nonce is included in the HKDF input indirectly through the
per-message encryption step. There is no long-lived symmetric session key
that accumulates nonce usage over time — every encryption derives a fresh
key from the X25519 DH output via HKDF (ADR-007).

## Crates

| Crate | Version | Role |
|---|---|---|
| `ed25519-dalek` | 2.1 | Ed25519 signing and verification |
| `x25519-dalek` | 2.0 | X25519 Diffie-Hellman key exchange |
| `chacha20poly1305` | 0.10 | Authenticated symmetric encryption |
| `hkdf` | 0.12 | Key derivation from raw DH output (RFC 5869) |
| `sha2` | 0.10 | SHA-256 hash function, used as HKDF PRF |
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

- The HKDF info label `"ripple-v1-message"` is intentionally versioned.
  If the key derivation scheme ever changes, bump the label to
  `"ripple-v2-message"` to ensure keys derived under different schemes
  never collide. See ADR-007.
- Namespace shared-key encryption (Phase 3) will add a separate symmetric
  encryption path for broadcast bundles within private namespaces.
