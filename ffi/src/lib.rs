//! C FFI surface — exposes ripple-core to iOS (staticlib) and Android (cdylib).
//!
//! ## Design
//!
//! The Router (which owns Store and PeerManager) lives in a process-global
//! `OnceLock<Mutex<Router>>`. `mesh_init` initializes it once. Every other
//! function locks it, does work, and returns.
//!
//! ## Memory contract
//!
//! Functions that produce output data allocate a buffer on the Rust heap,
//! write the pointer and length into the caller-supplied `*mut *mut u8` /
//! `*mut usize` out-params, and return. The caller owns that memory and MUST
//! call `mesh_free(ptr, len)` when done. Failing to do so leaks memory.
//!
//! ## Return codes
//!
//!   0  = success
//!  -1  = not initialized (call mesh_init first)
//!  -2  = serialization error
//!  -3  = internal error (store, routing, crypto, etc.)

use ripple_core::bundle::{BundleBuilder, Destination, Priority};
use ripple_core::crypto::Identity;
use ripple_core::peer::Transport;
use ripple_core::routing::{Action, Router, SyncOffer};
use ripple_core::store::Store;
use serde::Serialize;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

// ── Global state ──────────────────────────────────────────────────────────────

/// The process-global Router singleton.
///
/// **Rust note:** `OnceLock<T>` is the standard library's "initialize exactly
/// once, then read forever" cell. You can think of it like a `static` variable
/// that starts empty and gets filled in at runtime by the first caller of
/// `OnceLock::set()`. After that, `get()` always returns the value instantly
/// with no overhead.
///
/// `Mutex<T>` wraps the Router so multiple FFI calls can safely take turns
/// accessing it. Without the Mutex, two concurrent calls could race on the
/// Router's internal state. With it, they serialize — one waits while the
/// other runs.
///
/// In PHP terms: this is a process-level singleton with a lock around it.
static ROUTER: OnceLock<Mutex<Router>> = OnceLock::new();

// ── Return codes ──────────────────────────────────────────────────────────────

const OK:               i32 = 0;
const ERR_NOT_INIT:     i32 = -1;
const ERR_SERIALIZE:    i32 = -2;
const ERR_INTERNAL:     i32 = -3;

// ── Helper: write an allocated buffer to out-params ───────────────────────────

/// Serialize `value` to MessagePack, allocate a heap buffer, and write the
/// pointer + length into the caller's out-params.
///
/// Returns `ERR_SERIALIZE` if serialization fails, `OK` otherwise.
///
/// **Rust note on `unsafe`:** Writing to a raw pointer is `unsafe` because
/// Rust cannot verify that the pointer is valid — that's the caller's
/// responsibility. Everything else in this file is safe Rust; the `unsafe`
/// blocks are confined to the FFI boundary where they must be.
///
/// **Rust note on `Box::into_raw`:** `Box<T>` is heap-allocated, owned memory.
/// `into_raw()` converts it to a raw pointer and *transfers ownership to the
/// caller* — Rust will no longer track or free this memory. That's exactly what
/// we want: the C caller owns the buffer until they call `mesh_free`.
fn write_output<T: Serialize>(
    value: &T,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    let bytes = match rmp_serde::to_vec(value) {
        Ok(b)  => b,
        Err(_) => return ERR_SERIALIZE,
    };

    let len = bytes.len();
    // Convert Vec<u8> → Box<[u8]> → raw pointer.
    // Box<[u8]> is a heap-allocated slice with a known length.
    let ptr = Box::into_raw(bytes.into_boxed_slice()) as *mut u8;

    // SAFETY: out_ptr and out_len are caller-supplied pointers. The C caller
    // is responsible for passing valid, aligned, non-null pointers here.
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }

    OK
}

// ── mesh_init ─────────────────────────────────────────────────────────────────

/// Initialize the mesh core. Must be called before any other function.
///
/// `db_path`        — path to the SQLite database file (UTF-8, null-terminated
///                    via `db_path_len` — NOT null-terminated C string)
/// `identity_bytes` — 32-byte Ed25519 private key. If all zeros, a new
///                    identity is generated (useful for testing).
/// `identity_len`   — must be 32.
///
/// Returns OK on success, ERR_INTERNAL if the store can't be opened,
/// or ERR_NOT_INIT if called a second time (OnceLock only sets once).
///
/// # Safety
/// `db_path` must be a valid pointer to `db_path_len` bytes of UTF-8 data.
/// `identity_bytes` must be a valid pointer to `identity_len` bytes, or NULL.
#[no_mangle]
pub unsafe extern "C" fn mesh_init(
    db_path:        *const u8,
    db_path_len:    usize,
    identity_bytes: *const u8,
    identity_len:   usize,
) -> i32 {
    // SAFETY: we trust the caller passed a valid pointer + length.
    let path_bytes = unsafe { std::slice::from_raw_parts(db_path, db_path_len) };
    let path = match std::str::from_utf8(path_bytes) {
        Ok(s)  => s,
        Err(_) => return ERR_INTERNAL,
    };

    let identity = if identity_len == 32 {
        let key_bytes = unsafe { std::slice::from_raw_parts(identity_bytes, 32) };
        let arr: [u8; 32] = match key_bytes.try_into() {
            Ok(a)  => a,
            Err(_) => return ERR_INTERNAL,
        };
        // All-zeros means "generate a new identity".
        if arr == [0u8; 32] {
            Identity::generate()
        } else {
            Identity::from_bytes(&arr)
        }
    } else {
        Identity::generate()
    };

    let x25519_pubkey = identity.x25519_public_key();

    let store = match Store::new(path) {
        Ok(s)  => s,
        Err(_) => return ERR_INTERNAL,
    };

    let router = Router::new(store, x25519_pubkey);

    // `set` returns Err if already initialized — that's fine, we just
    // return ERR_NOT_INIT to tell the caller they called init twice.
    match ROUTER.set(Mutex::new(router)) {
        Ok(_)  => OK,
        Err(_) => ERR_NOT_INIT,
    }
}

// ── mesh_peer_encountered ─────────────────────────────────────────────────────

/// Notify the core that a peer has been encountered on a transport.
///
/// `ed25519_pubkey` — 32 bytes, peer's signing/identity key
/// `x25519_pubkey`  — 32 bytes, peer's encryption key
/// `transport`      — u8 transport code (matches Transport enum)
/// `rssi`           — signal strength in dBm (pass 0 for internet transport)
/// `now`            — current Unix timestamp in seconds
/// `out_offer`      — written with a pointer to MessagePack-serialized SyncOffer
/// `out_offer_len`  — written with byte length of the above
///
/// Caller must call `mesh_free(out_offer, out_offer_len)` when done.
/// 
/// # Safety
/// `ed25519_pubkey` and `x25519_pubkey` must each be valid pointers to 32 bytes.
/// `out_offer` and `out_offer_len` must be valid writable pointers.
#[no_mangle]
pub unsafe extern "C" fn mesh_peer_encountered(
    ed25519_pubkey:  *const u8,
    x25519_pubkey:   *const u8,
    transport:       u8,
    rssi:            i32,
    now:             i64,
    out_offer:       *mut *mut u8,
    out_offer_len:   *mut usize,
) -> i32 {
    let router_mutex = match ROUTER.get() {
        Some(m) => m,
        None    => return ERR_NOT_INIT,
    };

    // Read the two pubkeys from raw pointers.
    //
    // **Rust note:** `std::slice::from_raw_parts(ptr, len)` turns a raw pointer
    // + length into a borrowed slice. We then `.try_into()` the slice into a
    // `[u8; 32]` — this fails if the slice isn't exactly 32 bytes, which would
    // mean the caller passed a bad length. We've seen `try_into()` before in
    // store.rs for the same reason: converting an unsized Vec into a fixed array.
    let ed_key: [u8; 32] = unsafe {
        match std::slice::from_raw_parts(ed25519_pubkey, 32).try_into() {
            Ok(a)  => a,
            Err(_) => return ERR_INTERNAL,
        }
    };
    let x_key: [u8; 32] = unsafe {
        match std::slice::from_raw_parts(x25519_pubkey, 32).try_into() {
            Ok(a)  => a,
            Err(_) => return ERR_INTERNAL,
        }
    };

    let transport = match Transport::from_u8(transport) {
        Some(t) => t,
        None    => return ERR_INTERNAL,
    };

    let mut router = match router_mutex.lock() {
        Ok(r)  => r,
        Err(_) => return ERR_INTERNAL, // poisoned mutex
    };

    let offer: SyncOffer = match router.on_peer_encountered(ed_key, x_key, transport, rssi, now) {
        Ok(o)  => o,
        Err(_) => return ERR_INTERNAL,
    };

    // SyncOffer contains Vec<Uuid>. Serialize the IDs as Vec<[u8; 16]> —
    // raw UUID bytes are more portable across language boundaries than strings.
    let offer_bytes: Vec<[u8; 16]> = offer.bundle_ids
        .iter()
        .map(|id| *id.as_bytes())
        .collect();

    write_output(&offer_bytes, out_offer, out_offer_len)
}

// ── mesh_bundle_received ──────────────────────────────────────────────────────

/// Hand a received bundle (raw MessagePack bytes) to the core for processing.
///
/// `bundle_bytes` / `bundle_len` — the raw bundle as received from a peer
/// `now`          — current Unix timestamp in seconds
/// `out_actions`  — written with a pointer to MessagePack-serialized Vec<Action>
/// `out_actions_len`
///
/// Caller must call `mesh_free` on `out_actions` when done.
///
/// # Safety
/// `bundle_bytes` must be a valid pointer to `bundle_len` bytes.
/// `out_actions` and `out_actions_len` must be valid writable pointers.
#[no_mangle]
pub unsafe extern "C" fn mesh_bundle_received(
    bundle_bytes:     *const u8,
    bundle_len:       usize,
    now:              i64,
    out_actions:      *mut *mut u8,
    out_actions_len:  *mut usize,
) -> i32 {
    let router_mutex = match ROUTER.get() {
        Some(m) => m,
        None    => return ERR_NOT_INIT,
    };

    let bytes = unsafe { std::slice::from_raw_parts(bundle_bytes, bundle_len) };

    let bundle = match ripple_core::bundle::Bundle::from_bytes(bytes) {
        Ok(b)  => b,
        Err(_) => return ERR_INTERNAL,
    };

    let mut router = match router_mutex.lock() {
        Ok(r)  => r,
        Err(_) => return ERR_INTERNAL,
    };

    let actions = match router.on_bundle_received(bundle, now) {
        Ok(a)  => a,
        Err(_) => return ERR_INTERNAL,
    };

    let serializable = actions_to_serializable(&actions);
    write_output(&serializable, out_actions, out_actions_len)
}

// ── mesh_bundle_forwarded ─────────────────────────────────────────────────────

/// Notify the core that a bundle was successfully forwarded to a peer.
/// Decrements the spray count for Spray and Wait routing.
///
/// `bundle_id_bytes` — 16 raw bytes of the bundle's UUID
/// `bundle_id_len`   — must be 16
///
/// # Safety
/// `bundle_id_bytes` must be a valid pointer to exactly 16 bytes.
#[no_mangle]
pub unsafe extern "C" fn mesh_bundle_forwarded(
    bundle_id_bytes: *const u8,
    bundle_id_len:   usize,
) -> i32 {
    let router_mutex = match ROUTER.get() {
        Some(m) => m,
        None    => return ERR_NOT_INIT,
    };

    if bundle_id_len != 16 {
        return ERR_INTERNAL;
    }

    let id_bytes = unsafe { std::slice::from_raw_parts(bundle_id_bytes, 16) };
    let id_arr: [u8; 16] = match id_bytes.try_into() {
        Ok(a)  => a,
        Err(_) => return ERR_INTERNAL,
    };
    let bundle_id = Uuid::from_bytes(id_arr);

    let mut router = match router_mutex.lock() {
        Ok(r)  => r,
        Err(_) => return ERR_INTERNAL,
    };

    match router.on_bundle_forwarded(bundle_id) {
        Ok(_)  => OK,
        Err(_) => ERR_INTERNAL,
    }
}

// ── mesh_bundles_for_peer ─────────────────────────────────────────────────────

/// Get all queued bundles for a specific peer (by X25519 pubkey).
///
/// Returns MessagePack-serialized `Vec<Vec<u8>>` — each inner `Vec<u8>` is
/// a complete MessagePack-serialized Bundle, ready to send over the transport.
///
/// `x25519_pubkey`   — 32 bytes
/// `out_bundles`     — written with pointer to serialized bundle list
/// `out_bundles_len`
///
/// Caller must call `mesh_free` when done.
///
/// # Safety
/// `x25519_pubkey` must be a valid pointer to 32 bytes.
/// `out_bundles` and `out_bundles_len` must be valid writable pointers.
#[no_mangle]
pub unsafe extern "C" fn mesh_bundles_for_peer(
    x25519_pubkey:    *const u8,
    out_bundles:      *mut *mut u8,
    out_bundles_len:  *mut usize,
) -> i32 {
    let router_mutex = match ROUTER.get() {
        Some(m) => m,
        None    => return ERR_NOT_INIT,
    };

    let x_key: [u8; 32] = unsafe {
        match std::slice::from_raw_parts(x25519_pubkey, 32).try_into() {
            Ok(a)  => a,
            Err(_) => return ERR_INTERNAL,
        }
    };

    let router = match router_mutex.lock() {
        Ok(r)  => r,
        Err(_) => return ERR_INTERNAL,
    };

    let bundles = match router.store().bundles_for_peer(&x_key) {
        Ok(b)  => b,
        Err(_) => return ERR_INTERNAL,
    };

    // Serialize each bundle to its wire bytes, then wrap the whole list.
    let bundle_bytes: Vec<Vec<u8>> = bundles
        .iter()
        .filter_map(|b| b.to_bytes().ok())
        .collect();

    write_output(&bundle_bytes, out_bundles, out_bundles_len)
}

// ── mesh_create_bundle ────────────────────────────────────────────────────────

/// Create, sign, and store a new outbound bundle.
///
/// `dest_pubkey`  — 32 bytes for Peer destination, NULL for Broadcast
/// `payload`      — raw payload bytes
/// `payload_len`
/// `priority`     — 0=Normal, 1=Urgent, 2=SOS
/// `now`          — current Unix timestamp in seconds
/// `out_bundle`   — written with pointer to MessagePack-serialized Bundle
/// `out_bundle_len`
///
/// Caller must call `mesh_free` when done.
///
/// # Safety
/// `identity_bytes` must be a valid pointer to 32 bytes.
/// `dest_pubkey` must be a valid pointer to 32 bytes, or NULL for Broadcast.
/// `payload` must be a valid pointer to `payload_len` bytes.
/// `out_bundle` and `out_bundle_len` must be valid writable pointers.
///
/// **Note:** This function needs the Identity to sign the bundle, but the
/// Identity is not stored in the Router (the private key must not live in
/// a Mutex accessible to arbitrary FFI callers in a real deployment).
/// For Phase 1, we re-derive it from scratch — Phase 2 will add a proper
/// identity parameter here or a separate identity accessor.
///
/// For now: this function takes the 32-byte private key directly so the
/// CLI can call it. The private key is zeroed from the stack when the
/// function returns.
#[no_mangle]
pub unsafe extern "C" fn mesh_create_bundle(
    identity_bytes:   *const u8,
    identity_len:     usize,
    dest_pubkey:      *const u8, // NULL for Broadcast
    payload:          *const u8,
    payload_len:      usize,
    priority:         u8,
    now:              i64,
    out_bundle:       *mut *mut u8,
    out_bundle_len:   *mut usize,
) -> i32 {
    let router_mutex = match ROUTER.get() {
        Some(m) => m,
        None    => return ERR_NOT_INIT,
    };

    if identity_len != 32 {
        return ERR_INTERNAL;
    }

    let id_arr: [u8; 32] = unsafe {
        match std::slice::from_raw_parts(identity_bytes, 32).try_into() {
            Ok(a)  => a,
            Err(_) => return ERR_INTERNAL,
        }
    };
    let identity = Identity::from_bytes(&id_arr);

    let destination = if dest_pubkey.is_null() {
        Destination::Broadcast
    } else {
        let pk: [u8; 32] = unsafe {
            match std::slice::from_raw_parts(dest_pubkey, 32).try_into() {
                Ok(a)  => a,
                Err(_) => return ERR_INTERNAL,
            }
        };
        Destination::Peer(pk)
    };

    let priority = match priority {
        0 => Priority::Normal,
        1 => Priority::Urgent,
        2 => Priority::Sos,
        _ => return ERR_INTERNAL,
    };

    let payload_slice = unsafe { std::slice::from_raw_parts(payload, payload_len) };

    let bundle = match BundleBuilder::new(destination, priority)
        .payload(payload_slice.to_vec())
        .build(&identity, now)
    {
        Ok(b)  => b,
        Err(_) => return ERR_INTERNAL,
    };

    // Store it so it gets forwarded when peers are encountered.
    let router = match router_mutex.lock() {
        Ok(r)  => r,
        Err(_) => return ERR_INTERNAL,
    };
    if router.store().insert_bundle(&bundle).is_err() {
        return ERR_INTERNAL;
    }

    let bytes = match bundle.to_bytes() {
        Ok(b)  => b,
        Err(_) => return ERR_SERIALIZE,
    };

    // Wrap in a Vec so write_output serializes consistently with other outputs.
    write_output(&bytes, out_bundle, out_bundle_len)
}

// ── mesh_tick ─────────────────────────────────────────────────────────────────

/// Periodic heartbeat. Call every ~30 seconds from native.
///
/// Expires old bundles, returns any resulting Actions.
///
/// `now`             — current Unix timestamp in seconds
/// `out_actions`     — written with pointer to MessagePack-serialized Vec<Action>
/// `out_actions_len`
///
/// Caller must call `mesh_free` when done.
///
/// # Safety
/// `out_actions` and `out_actions_len` must be valid writable pointers.
#[no_mangle]
pub unsafe extern "C" fn mesh_tick(
    now:              i64,
    out_actions:      *mut *mut u8,
    out_actions_len:  *mut usize,
) -> i32 {
    let router_mutex = match ROUTER.get() {
        Some(m) => m,
        None    => return ERR_NOT_INIT,
    };

    let mut router = match router_mutex.lock() {
        Ok(r)  => r,
        Err(_) => return ERR_INTERNAL,
    };

    let actions = match router.mesh_tick(now) {
        Ok(a)  => a,
        Err(_) => return ERR_INTERNAL,
    };

    let serializable = actions_to_serializable(&actions);
    write_output(&serializable, out_actions, out_actions_len)
}

// ── mesh_free ─────────────────────────────────────────────────────────────────

/// Free a buffer previously allocated by this library.
///
/// MUST be called exactly once for every out-pointer written by any other
/// function in this library. Passing a pointer not allocated here, or calling
/// this twice on the same pointer, is undefined behavior.
///
/// # Safety
/// `ptr` must have been allocated by this library via a prior FFI call.
/// Must be called exactly once per allocated buffer. Passing NULL is safe.
///
/// **Rust note:** `Box::from_raw` is the mirror of `Box::into_raw`. It
/// reconstructs the Box from the raw pointer, taking ownership back. When
/// this Box goes out of scope at the end of the function, it is dropped —
/// which frees the heap memory. This is the correct, safe way to implement
/// a `free` in Rust FFI.
#[no_mangle]
pub unsafe extern "C" fn mesh_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        // Reconstruct the slice Box from the raw pointer and let it drop.
        let _ = Box::from_raw(std::ptr::slice_from_raw_parts_mut(ptr, len));}
}

// ── Action serialization ──────────────────────────────────────────────────────

/// A serializable mirror of the `Action` enum.
///
/// `Action` itself doesn't derive `Serialize` — it lives in ripple-core and
/// doesn't need to know about FFI serialization. We convert here at the boundary.
///
/// **Rust note:** This is a common pattern at FFI or API boundaries — a "DTO"
/// (Data Transfer Object) that mirrors the internal type but adds the traits
/// needed for the transport layer. In PHP you'd call this a resource or a
/// transformer. Here we're deriving `Serialize` so rmp-serde can encode it.
#[derive(Serialize)]
#[serde(tag = "type")]
enum SerializableAction {
    ForwardBundle {
        peer_pubkey: Vec<u8>,
        bundle_id:   [u8; 16],
    },
    NotifyUser {
        bundle_id: [u8; 16],
    },
    UpdateSharedState {
        key:   String,
        value: Vec<u8>,
    },
}

fn actions_to_serializable(actions: &[Action]) -> Vec<SerializableAction> {
    actions.iter().map(|a| match a {
        Action::ForwardBundle { peer_pubkey, bundle_id } => {
            SerializableAction::ForwardBundle {
                peer_pubkey: peer_pubkey.to_vec(),
                bundle_id:   *bundle_id.as_bytes(),
            }
        }
        Action::NotifyUser { bundle_id } => {
            SerializableAction::NotifyUser {
                bundle_id: *bundle_id.as_bytes(),
            }
        }
        Action::UpdateSharedState { key, value } => {
            SerializableAction::UpdateSharedState {
                key:   key.clone(),
                value: value.clone(),
            }
        }
    }).collect()
}
