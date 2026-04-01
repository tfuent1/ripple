//! `ripple daemon` — the main event loop.
//!
//! Two concurrent tokio tasks share a single Router behind Arc<Mutex<Router>>:
//!
//!   tick task   — fires every 30s, calls mesh_tick, handles Actions
//!   relay task  — fires every 30s (and immediately on start), polls the
//!                 rendezvous inbox and submits outbound bundles
//!
//! The core (Router, Store, Bundle) stays fully synchronous. Async is
//! confined to this file and relay.rs — only the network I/O layer.

use crate::relay;
use ripple_core::bundle::Bundle;
use ripple_core::crypto::Identity;
use ripple_core::routing::{Action, Router};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

// ── Entry point ───────────────────────────────────────────────────────────────

/// Start the daemon. Takes ownership of `router` and `identity`, runs forever.
///
/// **Rust ownership note:** `router` and `identity` are moved into this
/// function — the caller can no longer use them after this call. That's
/// intentional: the daemon owns the process state from this point forward.
///
/// **Async note:** This function is `async` because it uses `.await` inside.
/// The caller (`main.rs`) drives it with `Runtime::block_on(...)`, which
/// runs it to completion on the current thread.
pub async fn run(router: Router, identity: Identity, server_url: String) {
    // Wrap Router in Arc<Mutex<...>> so both tasks can share it.
    //
    // Arc  = "Atomic Reference Count" — shared ownership across tasks.
    //        Think of it as a thread-safe version of PHP's reference counting.
    // Mutex = mutual exclusion — only one task touches the Router at a time.
    //
    // Each `Arc::clone` creates a new *handle* to the same underlying data,
    // not a copy of the data itself. Cheap to clone, safe to share.
    let router = Arc::new(Mutex::new(router));

    let our_x25519 = identity.x25519_public_key();

    info!(
        "daemon started | Ed25519: {}",
        hex::encode(identity.public_key())
    );
    info!(
        "inbox key (X25519, share this): {}",
        hex::encode(our_x25519)
    );
    info!("rendezvous server: {server_url}");

    let router_tick  = Arc::clone(&router);
    let router_relay = Arc::clone(&router);
    let client       = reqwest::Client::new();
    let client_relay = client.clone();
    let server_relay = server_url.clone();

    // ── Tick task ─────────────────────────────────────────────────────────────
    //
    // `tokio::spawn` launches this closure as an independent concurrent task.
    // `async move` means: this is an async closure, and it *moves* (takes
    // ownership of) the captured variables into itself. After this line,
    // `router_tick` belongs to the task — we can no longer use it here.
    let tick_task = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            // Suspend this task until the next 30-second tick.
            // While suspended, tokio runs the relay task (and anything else).
            // This is the key difference from `std::thread::sleep` — no thread
            // is blocked; the thread is free to do other work.
            interval.tick().await;

            let now = unix_now();

            // Lock, work, unlock — the braces control the lock's lifetime.
            // When `r` goes out of scope at `}`, the MutexGuard is dropped,
            // which releases the lock. If we held the lock across an `.await`,
            // the relay task could never acquire it.
            let result = {
                let mut r = router_tick.lock().unwrap();
                r.mesh_tick(now)
            };

            match result {
                Ok(actions) => handle_actions(actions),
                Err(e)      => error!("mesh_tick error: {e}"),
            }
        }
    });

    // ── Relay task ────────────────────────────────────────────────────────────
    let relay_task = tokio::spawn(async move {
        // Poll once immediately so we don't wait 30s on startup.
        poll_and_relay(&router_relay, &client_relay, &server_relay, &our_x25519).await;

        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            poll_and_relay(&router_relay, &client_relay, &server_relay, &our_x25519).await;
        }
    });

    // Drive both tasks concurrently. Neither returns, so this runs forever.
    // If one task panics, tokio::join! surfaces the error here.
    let (t, r) = tokio::join!(tick_task, relay_task);
    if let Err(e) = t { error!("tick task panicked: {e}"); }
    if let Err(e) = r { error!("relay task panicked: {e}"); }
}

// ── Combined poll + relay outbound ────────────────────────────────────────────

/// One relay cycle: submit outbound bundles, then fetch and process inbound.
async fn poll_and_relay(
    router:    &Arc<Mutex<Router>>,
    client:    &reqwest::Client,
    server_url: &str,
    our_x25519: &[u8; 32],
) {
    relay_outbound(router, client, server_url).await;
    fetch_inbound(router, client, server_url, our_x25519).await;
}

// ── Relay outbound ────────────────────────────────────────────────────────────

/// Submit all undelivered bundles to the rendezvous server.
///
/// Phase 1: we submit everything. Phase 3 will filter by transport so we
/// skip bundles already synced over BLE.
async fn relay_outbound(
    router:    &Arc<Mutex<Router>>,
    client:    &reqwest::Client,
    server_url: &str,
) {
    // Collect bundles while holding the lock, then release it before
    // doing any I/O. You must never hold a Mutex lock across an .await —
    // if the task is suspended while holding the lock, no other task can
    // ever acquire it, which is a deadlock.
    let bundles: Vec<Bundle> = {
        let r = router.lock().unwrap();
        r.store().all_undelivered().unwrap_or_default()
    }; // <-- lock released here

    for bundle in bundles {
        match bundle.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = relay::submit_bundle(client, server_url, bytes).await {
                    warn!("relay submit failed for {}: {e}", bundle.id);
                } else {
                    info!("submitted bundle {} to relay", bundle.id);
                }
            }
            Err(e) => warn!("failed to serialize bundle {}: {e}", bundle.id),
        }
    }
}

// ── Fetch inbound ─────────────────────────────────────────────────────────────

/// Fetch bundles from our inbox, hand each to the Router, ack delivered ones.
async fn fetch_inbound(
    router:    &Arc<Mutex<Router>>,
    client:    &reqwest::Client,
    server_url: &str,
    our_x25519: &[u8; 32],
) {
    info!("polling inbox...");

    let blobs = match relay::fetch_inbox(client, server_url, our_x25519).await {
        Ok(b)  => b,
        Err(e) => { warn!("inbox fetch failed: {e}"); return; }
    };

    if blobs.is_empty() {
        info!("inbox empty");
        return;
    }

    info!("received {} bundle(s) from relay", blobs.len());

    let now = unix_now();

    for raw in &blobs {
        let bundle = match Bundle::from_bytes(raw) {
            Ok(b)  => b,
            Err(e) => { warn!("failed to deserialize bundle: {e}"); continue; }
        };

        let bundle_id = bundle.id;

        // Same pattern as relay_outbound: hold lock only for the synchronous
        // call, drop it before the next .await.
        let actions = {
            let mut r = router.lock().unwrap();
            r.on_bundle_received(bundle, now)
        };

        match actions {
            Ok(actions) => {
                handle_actions(actions);
                // Mark delivered locally so we don't process it again.
                {
                    let r = router.lock().unwrap();
                    if let Err(e) = r.store().mark_delivered(bundle_id) {
                        warn!("mark_delivered failed for {bundle_id}: {e}");
                    }
                }
                // Ack to relay so it removes the bundle from our inbox.
                if let Err(e) = relay::ack_bundle(client, server_url, bundle_id).await {
                    warn!("ack failed for {bundle_id}: {e}");
                }
            }
            Err(e) => error!("on_bundle_received error for {bundle_id}: {e}"),
        }
    }
}

// ── Action handler ────────────────────────────────────────────────────────────

fn handle_actions(actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::NotifyUser { bundle_id } => {
                // Phase 1: print to stdout. Phase 2: desktop notification.
                println!("📨  new message arrived (bundle {bundle_id})");
            }
            Action::ForwardBundle { peer_pubkey, bundle_id } => {
                // Internet forwarding happens automatically via relay_outbound.
                // BLE/WiFi Direct forwarding is Phase 2.
                info!(
                    "queued forward of {} to peer {}",
                    bundle_id,
                    hex::encode(&peer_pubkey[..8])
                );
            }
            Action::UpdateSharedState { key, value: _ } => {
                info!("shared state update: {key}");
            }
        }
    }
}

// ── Utility ───────────────────────────────────────────────────────────────────

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}
