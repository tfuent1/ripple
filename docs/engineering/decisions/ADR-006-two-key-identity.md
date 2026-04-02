# ADR-006: Two-Key Identity Model

## Status
Accepted

## Context
Each Ripple node needs to both sign bundles (proving authorship) and receive
encrypted direct messages (requiring a key exchange mechanism). Ed25519 is used
for signing. X25519 is used for Diffie-Hellman key exchange prior to symmetric
encryption. The question was whether to use one keypair for both or maintain
separate keys.

Ed25519 and X25519 are both based on Curve25519. The private scalar bytes are
compatible — an Ed25519 private key can be used directly as an X25519 static
secret. The public keys are different encodings of the same underlying point
(Edwards form vs Montgomery form) and are not interchangeable.

Options considered:

**Option A — One keypair with conversion**
Use the Ed25519 keypair for signing, and derive the X25519 public key from the
Ed25519 public key via the standard Edwards-to-Montgomery conversion. Signal,
Keybase, and libsodium all document and support this approach. Only one public
key needs to be advertised.

**Option B — Two keypairs, one persisted**
Maintain Ed25519 for signing and X25519 for encryption as separate conceptual
keys. Derive the X25519 secret from the Ed25519 private scalar at runtime —
never persist it separately. Peers advertise both public keys. Only the Ed25519
private key is stored.

## Decision
Option B — two-key model with a single persisted secret.

Using the same key material for two different cryptographic operations (signing
and key exchange) creates theoretical cross-protocol attack surface, even if no
practical attack is known today. Maintaining explicit separation provides a
cleaner security model and makes the distinct roles of each key visible in the
codebase and protocol.

The cost is that peers advertise two public keys instead of one. In practice
this means one extra `[u8; 32]` field on the `Peer` struct and in peer
handshake messages. Storage cost is zero — the X25519 secret is always derived
from the Ed25519 private scalar and never persisted.

## Consequences

**Positive:**
- Clear separation of signing key and encryption key — roles are explicit
- No cross-protocol key reuse risk
- Only one secret to persist, back up, and restore
- X25519 public key is always re-derivable from the stored Ed25519 private key

**Negative:**
- Peers must advertise two public keys (Ed25519 for signature verification,
  X25519 for encryption)
- `Destination::Peer` in the bundle schema holds the recipient's X25519 public
  key, not their Ed25519 identity key — callers must be careful to use the
  correct key at each call site
- Slightly larger peer handshake messages

## Implementation Note
The X25519 secret is derived as:
```rust
StaticSecret::from(signing_key.to_scalar_bytes())
```
`to_scalar_bytes()` (exposed via the `hazmat` feature of `ed25519-dalek`)
performs the SHA-512 expansion of the Ed25519 seed and returns the lower 32
bytes — the actual private scalar before clamping. `StaticSecret::from()`
then applies the RFC 7748 clamping step when constructing the key.

Using `signing_key.to_bytes()` (the raw 32-byte seed) directly would bypass
the SHA-512 expansion, producing a scalar that does not correspond to the
Ed25519 verifying key's Montgomery form. The correctness of this derivation
is verified by a unit test in `crypto.rs` that asserts the derived X25519
public key matches `verifying_key().to_montgomery()`.
