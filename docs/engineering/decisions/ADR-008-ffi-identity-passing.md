# ADR-008: FFI Identity Passing Strategy

## Status
Accepted

## Context
`mesh_create_bundle` in `ffi/src/lib.rs` requires the node's Ed25519 private
key to sign the outbound bundle. The private key does not live in the Router
singleton — the Router owns the Store and PeerManager, but deliberately holds
no key material. This means every call to `mesh_create_bundle` must receive
the private key somehow.

The Phase 1 implementation passes the raw 32-byte private key as a parameter
directly over the FFI boundary on every call:
```c
int32_t mesh_create_bundle(
    const uint8_t *identity_bytes,  // 32-byte Ed25519 private key
    uintptr_t      identity_len,
    ...
);
```

The key is wrapped in `Zeroizing<[u8; 32]>` on the Rust side so it is wiped
from the stack when the function returns. This is correct for Phase 1 but
raises a design question for Phase 2: should the native layer pass the key
on every call, or should the core hold it after `mesh_init`?

Options considered:

**Option A — Pass key on every call (current)**
The native layer (Swift Keychain / Android Keystore) holds the private key
and passes it on each signing call. The Rust core never retains key material
beyond the duration of a single FFI call.

Advantages: the private key never lives in the Rust heap or the
`OnceLock<Mutex<Router>>` singleton, which is accessible to any thread that
calls an FFI function. The key's lifetime is bounded to a single stack frame.

Disadvantages: the key crosses the FFI boundary (C ABI, raw pointer) on
every bundle creation. A memory-safe language boundary is crossed with a raw
pointer to sensitive material on each call.

**Option B — Store key in Router after mesh_init**
`mesh_init` accepts the private key and stores it inside the Router (wrapped
in `Zeroizing`). `mesh_create_bundle` then requires no key parameter.

Advantages: the key crosses the boundary only once at startup.

Disadvantages: the key lives in the `Mutex<Router>` for the entire process
lifetime. Any future FFI function that locks the Router can access key
material — the attack surface grows with every function added. On platforms
with multiple processes sharing memory (unlikely but not impossible in
container environments), this is a larger exposure.

**Option C — Separate identity FFI layer**
Add a second `OnceLock`-backed singleton (`IDENTITY`) that holds the
`Identity` in memory, initialized by `mesh_init`. `mesh_create_bundle`
locks `IDENTITY` only for the duration of the signing operation, then
releases it immediately — the Router lock and the Identity lock are never
held simultaneously.

Advantages: key is not re-passed on every call, but is isolated from the
Router singleton. Finer-grained locking.

Disadvantages: two global singletons to initialize and reason about.
Deadlock risk if a future refactor holds both locks simultaneously.

## Decision
**Option A for Phase 1. Evaluate Option C at the start of Phase 2.**

The Phase 1 call volume (CLI daemon creating a handful of bundles per session)
does not justify the complexity of Option C. The `Zeroizing` wrapper ensures
the key is wiped from the Rust stack immediately after use.

Before Phase 2 mobile integration begins, Option C should be evaluated. The
driver is call frequency — a mobile app creating many bundles per session
crosses the FFI boundary with raw key material on every one. Option C
eliminates that without expanding the Router's responsibility.

**The private key must never be stored in the Router.** This constraint holds
regardless of which option is chosen. The Router is a routing and persistence
layer, not a key management layer.

## Consequences

**Positive:**
- Phase 1: no additional complexity. Key is zeroized immediately after use.
- Clear decision point established before mobile work starts.
- The constraint against storing the key in the Router is explicitly documented.

**Negative:**
- Phase 1: the private key crosses the FFI boundary on every bundle creation.
- Option C adds a second singleton if adopted in Phase 2 — more state to
  reason about at startup.

## Implementation Note
The current Phase 1 implementation wraps the key in `Zeroizing<[u8; 32]>`
immediately on receipt:
```rust
let id_arr: zeroize::Zeroizing<[u8; 32]> = unsafe {
    match std::slice::from_raw_parts(identity_bytes, 32).try_into() {
        Ok(a) => zeroize::Zeroizing::new(a),
        Err(_) => return ERR_INTERNAL,
    }
};
let identity = Identity::from_bytes(&id_arr);
```

`Zeroizing<T>` overwrites the memory with zeros when it goes out of scope,
before deallocation. The key does not persist on the stack beyond the
function call.
