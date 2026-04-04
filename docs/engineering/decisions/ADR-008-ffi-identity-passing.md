# ADR-008: FFI Identity Passing Strategy

## Status
Accepted — updated Phase 2

## Context
`mesh_create_bundle` in `ffi/src/lib.rs` requires the node's Ed25519 private
key to sign the outbound bundle. The private key does not live in the Router
singleton — the Router owns the Store and PeerManager, but deliberately holds
no key material. This means every call to `mesh_create_bundle` must receive
the private key somehow.

The Phase 1 implementation passed the raw 32-byte private key as a parameter
directly over the FFI boundary on every call:
```c
int32_t mesh_create_bundle(
    const uint8_t *identity_bytes,  // 32-byte Ed25519 private key
    uintptr_t      identity_len,
    ...
);
```

The key was wrapped in `Zeroizing<[u8; 32]>` on the Rust side so it was wiped
from the stack when the function returned. This was correct for Phase 1 but
raised a design question for Phase 2: should the native layer pass the key
on every call, or should the core hold it after `mesh_init`?

Options considered:

**Option A — Pass key on every call (Phase 1)**
The native layer (Swift Keychain / Android Keystore) holds the private key
and passes it on each signing call. The Rust core never retains key material
beyond the duration of a single FFI call.

Advantages: the private key never lives in the Rust heap or the
`OnceLock<Mutex<Router>>` singleton, which is accessible to any thread that
calls an FFI function. The key's lifetime is bounded to a single stack frame.

Disadvantages: the key crosses the FFI boundary (C ABI, raw pointer) on
every bundle creation. A memory-safe language boundary is crossed with a raw
pointer to sensitive material on each call. Every future signing function
would also require this parameter, growing the call surface with key material.

**Option B — Store key in Router after mesh_init**
`mesh_init` accepts the private key and stores it inside the Router (wrapped
in `Zeroizing`). `mesh_create_bundle` then requires no key parameter.

Advantages: the key crosses the boundary only once at startup.

Disadvantages: the key lives in the `Mutex<Router>` for the entire process
lifetime. Any future FFI function that locks the Router can access key
material — the attack surface grows with every function added.

**Option C — Separate identity singleton (adopted)**
Add a second `OnceLock`-backed singleton (`IDENTITY`) that holds the full
`Identity` struct (Ed25519 signing key + derived X25519 keypair), initialized
by `mesh_init`. `mesh_create_bundle` locks `IDENTITY` only for the duration
of the signing operation, then releases it before acquiring `ROUTER` — the
two locks are never held simultaneously.

Advantages: the key crosses the boundary only once at startup. The `Identity`
struct is pre-computed — no re-derivation from raw bytes on every signing
call. Isolated from the Router — functions that lock the Router cannot access
key material. Cleaner `mesh_create_bundle` signature (removes two parameters).

Disadvantages: two global singletons to initialize and reason about. Deadlock
risk if a future refactor holds both locks simultaneously — mitigated by the
lock ordering rule below.

## Decision
**Option A for Phase 1. Option C adopted at Phase 2.**

Option C is implemented as of the Phase 2 kickoff refactor. The `IDENTITY`
singleton holds the full `Identity` struct. It is initialized once by
`mesh_init` alongside the Router.

**The private key must never be stored in the Router.** This constraint is
unchanged from Phase 1. The Router is a routing and persistence layer, not
a key management layer.

**Lock ordering rule:** Any code path that needs both singletons must acquire
`IDENTITY` first, complete the signing work, release the `IDENTITY` lock, then
acquire `ROUTER`. No function may hold both locks simultaneously. This is
enforced mechanically — the signing block is wrapped in `{ }` so the
`MutexGuard` is dropped at the closing brace before `ROUTER` is touched.

In practice no function currently needs both locks — signing uses `IDENTITY`
only, routing uses `ROUTER` only. The rule exists to stay safe as the function
surface grows.

## Consequences

**Positive:**
- The private key crosses the FFI boundary exactly once (at `mesh_init`).
- `mesh_create_bundle` signature is cleaner — two fewer parameters.
- No re-derivation cost on every signing call — `Identity` is pre-computed.
- Key material is isolated from the Router singleton.
- `ZeroizeOnDrop` on `Identity` ensures the key is wiped at process exit.

**Negative:**
- Two global singletons to reason about at startup.
- Lock ordering rule must be followed as new functions are added — violation
  risks deadlock.

## Implementation Note

The lock ordering rule is enforced by scoping:
```rust
// Acquire IDENTITY, sign, release — then acquire ROUTER.
let bundle = {
    let identity = identity_mutex.lock().unwrap();
    BundleBuilder::new(destination, priority)
        .payload(payload)
        .build(&identity, now)
    // MutexGuard dropped here — IDENTITY lock released.
};

// IDENTITY lock is released. Safe to acquire ROUTER.
let router = router_mutex.lock().unwrap();
router.queue_outbound(&bundle)?;
```

The `IDENTITY` singleton is initialized before `ROUTER` in `mesh_init`,
consistent with the lock ordering rule:
```rust
IDENTITY.set(Mutex::new(identity)).map_err(|_| ERR_NOT_INIT)?;
ROUTER.set(Mutex::new(router)).map_err(|_| ERR_NOT_INIT)?;
```
