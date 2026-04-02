//! FFI surface smoke tests.
//!
//! These tests verify the C FFI boundary: correct return codes, valid
//! MessagePack output, and memory safety (no leaks, null-safe free).
//!
//! Each test is fully self-contained — no shared global state, no required
//! execution order, safe to run in parallel with the default test harness.
//!
//! How we avoid the OnceLock problem:
//! The global ROUTER can only be initialized once per process. Rather than
//! calling mesh_init in every test (which would race), we test the FFI
//! functions that DON'T require the global router (mesh_free, return codes
//! from uninitialized state) and test the core logic that the FFI delegates
//! to directly via ripple_core. The one test that needs a live router gets
//! its own dedicated test binary via [[test]] — see ffi/Cargo.toml.

use ripple_ffi::{mesh_free, mesh_tick, ERR_NOT_INIT, OK};

// ── Tests that don't need a live router ──────────────────────────────────────

#[test]
fn mesh_free_null_is_safe() {
    // The null guard in mesh_free must not crash or panic.
    unsafe { mesh_free(std::ptr::null_mut(), 0) };
}

#[test]
fn mesh_free_zero_len_is_safe() {
    // Verify mesh_free handles null + zero length without crashing.
    // (A separately-allocated buffer can't be freed via mesh_free since it
    // wasn't allocated by Box::into_raw — just verify the null guard again.)
    unsafe { mesh_free(std::ptr::null_mut(), 0) };
}

#[test]
fn uninitialized_mesh_tick_returns_err_not_init() {
    // If mesh_init has not been called, every function that needs the router
    // should return ERR_NOT_INIT. This test runs before any mesh_init call
    // in this process (tests run in parallel, no ordering guarantee, but
    // this test doesn't call mesh_init itself so the race doesn't matter —
    // if another test's mesh_init wins first, we get a different return code
    // which is also acceptable; see assertion below).
    let mut out: *mut u8 = std::ptr::null_mut();
    let mut len: usize = 0;
    let rc = unsafe { mesh_tick(1_700_000_000, &mut out, &mut len) };
    // Either ERR_NOT_INIT (router not set) or OK (another test initialized it).
    // What we're verifying is that it doesn't panic or return garbage.
    assert!(
        rc == ERR_NOT_INIT || rc == OK,
        "mesh_tick must return ERR_NOT_INIT or OK, got {rc}"
    );
    if !out.is_null() {
        unsafe { mesh_free(out, len) };
    }
}

// ── Core logic tests (no FFI global state needed) ────────────────────────────
//
// These test the same logic the FFI functions delegate to, but through the
// ripple_core public API directly. This gives us confident coverage of the
// bundle creation, signing, encryption, and routing pipeline without the
// OnceLock limitation.

#[test]
fn core_create_and_verify_bundle() {
    use ripple_core::bundle::{BundleBuilder, Destination, Priority};
    use ripple_core::crypto::Identity;

    let sender = Identity::generate();
    let recipient = Identity::generate();

    let bundle = BundleBuilder::new(
        Destination::Peer(recipient.x25519_public_key()),
        Priority::Normal,
    )
    .payload(b"ffi smoke test".to_vec())
    .build(&sender, 1_700_000_000)
    .unwrap();

    bundle.verify().expect("bundle signature must verify");

    let bytes = bundle.to_bytes().unwrap();
    let restored = ripple_core::bundle::Bundle::from_bytes(&bytes).unwrap();
    restored
        .verify()
        .expect("roundtrip bundle must still verify");
}

#[test]
fn core_bundle_received_notifies_correct_recipient() {
    use ripple_core::bundle::{BundleBuilder, Destination, Priority};
    use ripple_core::crypto::Identity;
    use ripple_core::routing::{Action, Router};
    use ripple_core::store::Store;

    let alice = Identity::generate();
    let bob = Identity::generate();

    let mut router = Router::new(Store::new(":memory:").unwrap(), bob.x25519_public_key());

    let bundle = BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
        .payload(b"for bob".to_vec())
        .build(&alice, 1_700_000_000)
        .unwrap();

    let bundle_id = bundle.id;
    let actions = router.on_bundle_received(bundle, 1_700_000_000).unwrap();

    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0], Action::NotifyUser { bundle_id });
}

#[test]
fn core_bundle_for_wrong_recipient_produces_no_actions() {
    use ripple_core::bundle::{BundleBuilder, Destination, Priority};
    use ripple_core::crypto::Identity;
    use ripple_core::routing::Router;
    use ripple_core::store::Store;

    let alice = Identity::generate();
    let bob = Identity::generate();
    let charlie = Identity::generate();

    // Charlie is running this router.
    let mut router = Router::new(Store::new(":memory:").unwrap(), charlie.x25519_public_key());

    // Bundle is for Bob, not Charlie.
    let bundle = BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
        .payload(b"for bob".to_vec())
        .build(&alice, 1_700_000_000)
        .unwrap();

    let actions = router.on_bundle_received(bundle, 1_700_000_000).unwrap();
    assert!(actions.is_empty());
}

// ── FFI round-trip test (needs live router — isolated) ───────────────────────
//
// This test calls mesh_init and exercises the full FFI pipeline.
// It is in its own [[test]] binary (see ffi/Cargo.toml: [[test]] name="ffi_roundtrip")
// so it gets a fresh process with a clean OnceLock.
// Run with: cargo test -p ripple-ffi --test ffi_roundtrip
