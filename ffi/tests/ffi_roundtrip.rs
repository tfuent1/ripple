//! Full FFI round-trip test.
//!
//! This test binary calls mesh_init and exercises mesh_create_bundle →
//! mesh_bundle_received end-to-end through the C FFI surface.
//!
//! It lives in its own [[test]] binary so it gets a fresh process with a
//! clean OnceLock<Mutex<Router>>. This means it can call mesh_init exactly
//! once without racing with other tests.
//!
//! Run: cargo test -p ripple-ffi --test ffi_roundtrip

use ripple_ffi::{
    mesh_bundle_received, mesh_create_bundle, mesh_free, mesh_init, mesh_tick, ERR_NOT_INIT, OK,
};

fn init() -> [u8; 32] {
    let identity = ripple_core::crypto::Identity::generate();
    let private_bytes = identity.to_private_bytes();
    let db_path = ":memory:";

    let rc = unsafe {
        mesh_init(
            db_path.as_ptr(),
            db_path.len(),
            private_bytes.as_ptr(),
            private_bytes.len(),
        )
    };
    assert_eq!(rc, OK, "mesh_init must return OK on first call");
    private_bytes
}

fn unwrap_msgpack_bytes(ptr: *mut u8, len: usize) -> Vec<u8> {
    assert!(!ptr.is_null());
    let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
    rmp_serde::from_slice::<Vec<u8>>(slice).expect("must be MessagePack Vec<u8>")
}

#[test]
fn full_ffi_roundtrip() {
    let _private_bytes = init();

    // Create a bundle from a fresh sender to a fresh recipient.
    let sender = ripple_core::crypto::Identity::generate();
    let recipient = ripple_core::crypto::Identity::generate();
    let private_bytes = sender.to_private_bytes();
    let dest = recipient.x25519_public_key();
    let payload = b"full ffi roundtrip";
    let now: i64 = 1_700_000_000;

    let mut out_bundle: *mut u8 = std::ptr::null_mut();
    let mut out_len: usize = 0;

    let rc = unsafe {
        mesh_create_bundle(
            private_bytes.as_ptr(),
            private_bytes.len(),
            dest.as_ptr(),
            payload.as_ptr(),
            payload.len(),
            0, // Normal
            now,
            &mut out_bundle,
            &mut out_len,
        )
    };

    assert_eq!(rc, OK, "mesh_create_bundle must return OK");
    let bundle_bytes = unwrap_msgpack_bytes(out_bundle, out_len);
    unsafe { mesh_free(out_bundle, out_len) };

    // Verify the bundle parses and has a valid signature.
    let bundle = ripple_core::bundle::Bundle::from_bytes(&bundle_bytes).unwrap();
    bundle
        .verify()
        .expect("created bundle must have valid signature");

    // Hand it to on_bundle_received.
    let mut out_actions: *mut u8 = std::ptr::null_mut();
    let mut actions_len: usize = 0;

    let rc = unsafe {
        mesh_bundle_received(
            bundle_bytes.as_ptr(),
            bundle_bytes.len(),
            now,
            &mut out_actions,
            &mut actions_len,
        )
    };

    assert_eq!(rc, OK, "mesh_bundle_received must return OK");
    assert!(!out_actions.is_null());
    unsafe { mesh_free(out_actions, actions_len) };

    // mesh_tick must also work after init.
    let mut out_tick: *mut u8 = std::ptr::null_mut();
    let mut tick_len: usize = 0;
    let rc = unsafe { mesh_tick(now, &mut out_tick, &mut tick_len) };
    assert_eq!(rc, OK);
    unsafe { mesh_free(out_tick, tick_len) };

    // Second mesh_init must return ERR_NOT_INIT.
    let identity2 = ripple_core::crypto::Identity::generate();
    let pk2 = identity2.to_private_bytes();
    let db = ":memory:";
    let rc = unsafe { mesh_init(db.as_ptr(), db.len(), pk2.as_ptr(), pk2.len()) };
    assert_eq!(
        rc, ERR_NOT_INIT,
        "second mesh_init must return ERR_NOT_INIT"
    );
}
