//! Integration tests for the relay HTTP transport.
//!
//! Spins up a real rendezvous server in-process on a random OS-assigned port
//! and exercises submit → poll → ack against it. This is the only test that
//! covers the full relay round-trip — the path that was the only working
//! transport in Phase 1.
//!
//! Using a real server rather than a mock catches actual HTTP encoding issues
//! (base64, status codes, JSON shape) that mocks tend to hide.

use ripple_cli::utils::unix_now;
use ripple_core::bundle::{Bundle, BundleBuilder, Destination, Priority};
use ripple_core::crypto::Identity;
use ripple_rendezvous::db::Db;
use ripple_rendezvous::server::{build_router, AppState};
use std::net::SocketAddr;

// ── Server helper ─────────────────────────────────────────────────────────────

/// Spin up an in-process rendezvous server on a random port.
/// Returns the base URL, e.g. "http://127.0.0.1:54321".
///
/// Port 0 tells the OS to pick a free port — avoids collisions when tests
/// run in parallel. Each test gets its own server and in-memory DB, so
/// tests are fully isolated.
async fn spawn_server() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();

    let db = Db::open(":memory:").unwrap();
    let state = AppState::new(db, 64 * 1024);
    let router = build_router(state);

    tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });

    // Wait until the server is actually accepting connections before returning.
    // tokio::spawn returns immediately — without this, tests can fire their
    // first request before axum's accept loop has started, getting a
    // "connection refused" that reqwest silently swallows, leaving the inbox
    // empty when we poll.
    let url = format!("http://{addr}");
    let client = reqwest::Client::new();
    for _ in 0..20 {
        if client.get(format!("{url}/inbox/00")).send().await.is_ok() {
            return url;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("rendezvous server did not become ready within 200ms");
}

/// Build a signed bundle from `sender` to `recipient_x25519` for use in tests.
fn make_bundle(sender: &Identity, recipient_x25519: [u8; 32]) -> Vec<u8> {
    BundleBuilder::new(Destination::Peer(recipient_x25519), Priority::Normal)
        .payload(b"relay integration test".to_vec())
        .build(sender, unix_now())
        .unwrap()
        .to_bytes()
        .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn submit_bundle_and_fetch_from_inbox() {
    let server_url = spawn_server().await;
    let client = reqwest::Client::new();

    let alice = Identity::generate();
    let bob = Identity::generate();
    let bundle_bytes = make_bundle(&alice, bob.x25519_public_key());

    ripple_cli::relay::submit_bundle(&client, &server_url, bundle_bytes.clone())
        .await
        .expect("submit_bundle should succeed");

    let inbox = ripple_cli::relay::fetch_inbox(&client, &server_url, &bob.x25519_public_key())
        .await
        .expect("fetch_inbox should succeed");

    assert_eq!(inbox.len(), 1, "inbox should contain exactly one bundle");
    assert_eq!(
        inbox[0], bundle_bytes,
        "received bytes must match submitted bytes"
    );
}

#[tokio::test]
async fn ack_removes_bundle_from_inbox() {
    let server_url = spawn_server().await;
    let client = reqwest::Client::new();

    let alice = Identity::generate();
    let bob = Identity::generate();
    let raw = make_bundle(&alice, bob.x25519_public_key());
    let bundle_id = Bundle::from_bytes(&raw).unwrap().id;

    ripple_cli::relay::submit_bundle(&client, &server_url, raw)
        .await
        .unwrap();

    // Verify it arrived.
    let before = ripple_cli::relay::fetch_inbox(&client, &server_url, &bob.x25519_public_key())
        .await
        .unwrap();
    assert_eq!(before.len(), 1);

    // Ack it.
    ripple_cli::relay::ack_bundle(&client, &server_url, bundle_id)
        .await
        .expect("ack_bundle should succeed");

    // Should be gone.
    let after = ripple_cli::relay::fetch_inbox(&client, &server_url, &bob.x25519_public_key())
        .await
        .unwrap();
    assert!(after.is_empty(), "inbox must be empty after ack");
}

#[tokio::test]
async fn ack_nonexistent_bundle_does_not_fail() {
    // Acking a bundle that was already deleted (or never existed) must not
    // return an error. At-least-once delivery means we may ack twice if the
    // daemon restarts between processing and acking — both must succeed.
    let server_url = spawn_server().await;
    let client = reqwest::Client::new();

    let fake_id = uuid::Uuid::new_v4();
    ripple_cli::relay::ack_bundle(&client, &server_url, fake_id)
        .await
        .expect("acking a nonexistent bundle should not fail");
}

#[tokio::test]
async fn duplicate_submit_produces_single_inbox_entry() {
    // The rendezvous server uses INSERT OR IGNORE — submitting the same bundle
    // twice must not create two inbox entries.
    let server_url = spawn_server().await;
    let client = reqwest::Client::new();

    let alice = Identity::generate();
    let bob = Identity::generate();
    let raw = make_bundle(&alice, bob.x25519_public_key());

    ripple_cli::relay::submit_bundle(&client, &server_url, raw.clone())
        .await
        .unwrap();
    ripple_cli::relay::submit_bundle(&client, &server_url, raw)
        .await
        .unwrap();

    let inbox = ripple_cli::relay::fetch_inbox(&client, &server_url, &bob.x25519_public_key())
        .await
        .unwrap();
    assert_eq!(
        inbox.len(),
        1,
        "duplicate submit must not produce two inbox entries"
    );
}

#[tokio::test]
async fn empty_inbox_returns_empty_vec() {
    let server_url = spawn_server().await;
    let client = reqwest::Client::new();

    let nobody = Identity::generate();
    let inbox = ripple_cli::relay::fetch_inbox(&client, &server_url, &nobody.x25519_public_key())
        .await
        .unwrap();

    assert!(inbox.is_empty());
}

#[tokio::test]
async fn server_rejects_tampered_bundle() {
    // The rendezvous server validates bundle signatures on insert (post-M1.8
    // hardening). A tampered payload must be rejected with an error.
    let server_url = spawn_server().await;
    let client = reqwest::Client::new();

    let alice = Identity::generate();
    let bob = Identity::generate();

    // Build a valid bundle, then corrupt the payload after signing.
    let mut bundle =
        BundleBuilder::new(Destination::Peer(bob.x25519_public_key()), Priority::Normal)
            .payload(b"legitimate".to_vec())
            .build(&alice, unix_now())
            .unwrap();
    bundle.payload = b"tampered".to_vec();
    let tampered_bytes = bundle.to_bytes().unwrap();

    // submit_bundle returns RelayError::ServerError for non-2xx responses.
    let result = ripple_cli::relay::submit_bundle(&client, &server_url, tampered_bytes).await;
    assert!(
        result.is_err(),
        "server must reject a bundle with an invalid signature"
    );
}
