//! Integration tests for bundle TTL expiry via mesh_tick.
//!
//! These tests verify that the expiry pipeline works correctly end-to-end:
//! Router::mesh_tick → Store::expire_bundles → bundles are gone from queries.
//! They also verify the SOS exception — epidemic bundles must never be expired
//! regardless of how much time passes.

use ripple_core::bundle::{BundleBuilder, Destination, Priority};
use ripple_core::crypto::Identity;
use ripple_core::routing::Router;
use ripple_core::store::Store;

const NOW: i64 = 1_700_000_000;

fn make_router(identity: &Identity) -> Router {
    let store = Store::new(":memory:").unwrap();
    Router::new(store, identity.x25519_public_key())
}

// ── Normal expiry ─────────────────────────────────────────────────────────────

#[test]
fn normal_bundle_expires_after_24h() {
    let identity = Identity::generate();
    let mut router = make_router(&identity);

    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"temporary".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let bundle_id = bundle.id;
    router.store().insert_bundle(&bundle).unwrap();

    // Not expired yet.
    assert!(router.store().get_bundle(bundle_id).unwrap().is_some());

    // Tick just before expiry — bundle survives.
    router.mesh_tick(NOW + 24 * 3600 - 1).unwrap();
    assert!(router.store().get_bundle(bundle_id).unwrap().is_some());

    // Tick at expiry — bundle is gone.
    router.mesh_tick(NOW + 24 * 3600).unwrap();
    assert!(
        router.store().get_bundle(bundle_id).unwrap().is_none(),
        "normal bundle must be expired after 24h"
    );
}

#[test]
fn urgent_bundle_expires_after_12h() {
    let identity = Identity::generate();
    let mut router = make_router(&identity);

    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Urgent)
        .payload(b"urgent".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let bundle_id = bundle.id;
    router.store().insert_bundle(&bundle).unwrap();

    // Tick just before 12h — survives.
    router.mesh_tick(NOW + 12 * 3600 - 1).unwrap();
    assert!(router.store().get_bundle(bundle_id).unwrap().is_some());

    // Tick at 12h — gone.
    router.mesh_tick(NOW + 12 * 3600).unwrap();
    assert!(
        router.store().get_bundle(bundle_id).unwrap().is_none(),
        "urgent bundle must be expired after 12h"
    );
}

// ── SOS must never expire ─────────────────────────────────────────────────────

#[test]
fn sos_bundle_survives_far_future_tick() {
    let identity = Identity::generate();
    let mut router = make_router(&identity);

    let sos = BundleBuilder::new(Destination::Broadcast, Priority::Sos)
        .payload(b"mayday".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let sos_id = sos.id;
    router.store().insert_bundle(&sos).unwrap();

    // Tick 100 years into the future.
    router.mesh_tick(NOW + 100 * 365 * 24 * 3600).unwrap();

    assert!(
        router.store().get_bundle(sos_id).unwrap().is_some(),
        "SOS bundle must never be expired by mesh_tick"
    );
}

#[test]
fn sos_survives_while_normal_bundles_expire() {
    let identity = Identity::generate();
    let mut router = make_router(&identity);

    let normal = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"expires".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let sos = BundleBuilder::new(Destination::Broadcast, Priority::Sos)
        .payload(b"never expires".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let normal_id = normal.id;
    let sos_id = sos.id;

    router.store().insert_bundle(&normal).unwrap();
    router.store().insert_bundle(&sos).unwrap();

    // Both present before expiry.
    assert!(router.store().get_bundle(normal_id).unwrap().is_some());
    assert!(router.store().get_bundle(sos_id).unwrap().is_some());

    // Tick past 24h — normal expires, SOS survives.
    router.mesh_tick(NOW + 25 * 3600).unwrap();

    assert!(
        router.store().get_bundle(normal_id).unwrap().is_none(),
        "normal bundle must expire"
    );
    assert!(
        router.store().get_bundle(sos_id).unwrap().is_some(),
        "SOS bundle must survive"
    );
}

// ── Multiple bundles ──────────────────────────────────────────────────────────

#[test]
fn multiple_expired_bundles_all_cleaned_up() {
    let identity = Identity::generate();
    let mut router = make_router(&identity);

    // Insert 10 normal bundles.
    let ids: Vec<_> = (0..10)
        .map(|i| {
            let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
                .payload(format!("bundle {i}").into_bytes())
                .build(&identity, NOW)
                .unwrap();
            let id = bundle.id;
            router.store().insert_bundle(&bundle).unwrap();
            id
        })
        .collect();

    // All present before expiry.
    for id in &ids {
        assert!(router.store().get_bundle(*id).unwrap().is_some());
    }

    // Tick past 24h — all gone.
    router.mesh_tick(NOW + 25 * 3600).unwrap();

    for id in &ids {
        assert!(
            router.store().get_bundle(*id).unwrap().is_none(),
            "bundle {id} should have been expired"
        );
    }
}

#[test]
fn bundles_created_at_different_times_expire_independently() {
    let identity = Identity::generate();
    let mut router = make_router(&identity);

    // Bundle A created at NOW — expires at NOW + 24h.
    let bundle_a = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"old".to_vec())
        .build(&identity, NOW)
        .unwrap();

    // Bundle B created 12 hours later — expires at NOW + 36h.
    let bundle_b = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"newer".to_vec())
        .build(&identity, NOW + 12 * 3600)
        .unwrap();

    let id_a = bundle_a.id;
    let id_b = bundle_b.id;

    router.store().insert_bundle(&bundle_a).unwrap();
    router.store().insert_bundle(&bundle_b).unwrap();

    // Tick at NOW + 25h — A expired, B still alive.
    router.mesh_tick(NOW + 25 * 3600).unwrap();

    assert!(
        router.store().get_bundle(id_a).unwrap().is_none(),
        "bundle A must be expired at NOW + 25h"
    );
    assert!(
        router.store().get_bundle(id_b).unwrap().is_some(),
        "bundle B must still be alive at NOW + 25h"
    );

    // Tick at NOW + 37h — B also expired.
    router.mesh_tick(NOW + 37 * 3600).unwrap();

    assert!(
        router.store().get_bundle(id_b).unwrap().is_none(),
        "bundle B must be expired at NOW + 37h"
    );
}

// ── Delivered bundles ─────────────────────────────────────────────────────────

#[test]
fn delivered_bundles_are_not_visible_regardless_of_expiry() {
    // mark_delivered hides a bundle from get_bundle regardless of its TTL.
    // Expiry is a separate cleanup mechanism — both should result in the
    // bundle being invisible to queries.
    let identity = Identity::generate();
    let mut router = make_router(&identity);

    let bundle = BundleBuilder::new(Destination::Broadcast, Priority::Normal)
        .payload(b"delivered".to_vec())
        .build(&identity, NOW)
        .unwrap();

    let bundle_id = bundle.id;
    router.store().insert_bundle(&bundle).unwrap();
    router.store().mark_delivered(bundle_id).unwrap();

    // Delivered bundles don't appear in get_bundle.
    assert!(router.store().get_bundle(bundle_id).unwrap().is_none());

    // Ticking past expiry doesn't cause any issues — row still exists in
    // SQLite (delivered = 1) until a future cleanup pass removes it.
    router.mesh_tick(NOW + 25 * 3600).unwrap();

    // Still not visible — mark_delivered hides it regardless.
    assert!(router.store().get_bundle(bundle_id).unwrap().is_none());
}
