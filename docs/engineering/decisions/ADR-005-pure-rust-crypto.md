# ADR-005: Pure Rust Crypto (no libsodium)

## Status
Accepted

## Context
The original architecture specified libsodium via the `sodiumoxide` crate for
all cryptographic operations. During Milestone 1.1 implementation, sodiumoxide
was found to be unmaintained. Additionally, libsodium is a C library that must
be compiled and linked, which adds significant complexity to cross-compilation
for iOS (XCFramework) and Android (multiple ABIs via JNI).

Options considered:

**Option A — sodiumoxide**
The original choice. Wraps libsodium, which is battle-tested and widely deployed.
Rejected because sodiumoxide is unmaintained and the C build dependency creates
real cross-compilation friction.

**Option B — aws-lc-rs**
Amazon's maintained fork of BoringSSL with Rust bindings. Well maintained but
still a C dependency, and heavier than needed for this use case.

**Option C — Pure Rust crates (ed25519-dalek, x25519-dalek, chacha20poly1305)**
Actively maintained pure Rust implementations. No C dependency. Eliminate
cross-compilation complexity entirely. Part of the RustCrypto project, which is
the de facto standard for cryptographic primitives in the Rust ecosystem.

## Decision
Option C — pure Rust crates with no C dependency.

| Operation | Crate |
|---|---|
| Ed25519 signing / verification | `ed25519-dalek` |
| X25519 key exchange | `x25519-dalek` |
| Authenticated encryption | `chacha20poly1305` |
| Secure memory zeroing | `zeroize` |

The encryption scheme changes from XSalsa20-Poly1305 (libsodium default) to
ChaCha20-Poly1305. Both are secure authenticated encryption schemes built on
the same underlying primitives. ChaCha20-Poly1305 is standardized in RFC 8439
and is the modern choice.

## Consequences

**Positive:**
- No C toolchain dependency for iOS or Android cross-compilation
- All crates actively maintained under the RustCrypto umbrella
- ChaCha20-Poly1305 is an IETF standard (RFC 8439)
- `zeroize` integration ensures private key material is wiped from memory on drop

**Negative:**
- Wire-incompatible with any future implementation that uses libsodium's
  XSalsa20-Poly1305 — if other clients are ever built against libsodium directly,
  they must use ChaCha20-Poly1305 to interoperate
- Pure Rust implementations have not had the same volume of real-world deployment
  as libsodium, though they are formally audited
