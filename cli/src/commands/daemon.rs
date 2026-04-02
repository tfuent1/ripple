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
use ripple_core::crypto::{self, Identity};
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
pub async fn run(router: Router, identity: Identity, server_url: String, quiet: bool) {
    // INVARIANT: rusqlite::Connection is !Send, which means Store (and therefore
    // Router) is also !Send in isolation. Wrapping in Mutex<T> makes the whole
    // Arc<Mutex<Router>> Send + Sync — the lock guarantees only one thread
    // touches the Connection at a time, satisfying SQLite's thread-safety
    // requirements.
    //
    // This is safe here because both tokio tasks (tick_task and relay_task)
    // acquire the lock only for synchronous calls and release it immediately
    // before any .await point. The Connection is never accessed concurrently.
    //
    // If a third task is added, or this codebase migrates to tokio-rusqlite
    // for async DB access in Phase 2, revisit this invariant.
    let router = Arc::new(Mutex::new(router));

    // Identity is read-only after startup — no mutation needed, so no Mutex.
    // But tokio::spawn requires everything it captures to be 'static (i.e.
    // owned, not borrowed). Wrapping in Arc gives each task an owned handle
    // to the same Identity without copying the keypair bytes.
    let identity = Arc::new(identity);

    let our_x25519 = identity.x25519_public_key();

    println!("Ed25519 : {}", hex::encode(identity.public_key()));
    println!(
        "X25519  : {} (share this — it's your inbox key)",
        hex::encode(our_x25519)
    );
    if !quiet {
        info!("rendezvous server: {server_url}");
    }

    let router_tick = Arc::clone(&router);
    let router_relay = Arc::clone(&router);
    let identity_tick = Arc::clone(&identity);
    let identity_relay = Arc::clone(&identity);
    let client = reqwest::Client::new();
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
            interval.tick().await;

            let now = unix_now();

            let result = {
                let mut r = router_tick.lock().unwrap();
                r.mesh_tick(now)
            };

            match result {
                Ok(actions) => handle_actions(actions, &router_tick, &identity_tick, quiet),
                Err(e) => error!("mesh_tick error: {e}"),
            }
        }
    });

    // ── Relay task ────────────────────────────────────────────────────────────
    let relay_task = tokio::spawn(async move {
        // Poll once immediately so we don't wait 30s on startup.
        poll_and_relay(
            &router_relay,
            &client_relay,
            &server_relay,
            &our_x25519,
            &identity_relay,
            quiet,
        )
        .await;

        let mut interval = tokio::time::interval(Duration::from_secs(30));
        loop {
            interval.tick().await;
            poll_and_relay(
                &router_relay,
                &client_relay,
                &server_relay,
                &our_x25519,
                &identity_relay,
                quiet,
            )
            .await;
        }
    });

    // Drive both tasks concurrently. Neither returns, so this runs forever.
    // If one task panics, tokio::join! surfaces the error here.
    let (t, r) = tokio::join!(tick_task, relay_task);
    if let Err(e) = t {
        error!("tick task panicked: {e}");
    }
    if let Err(e) = r {
        error!("relay task panicked: {e}");
    }
}

// ── Combined poll + relay outbound ────────────────────────────────────────────

/// One relay cycle: submit outbound bundles, then fetch and process inbound.
async fn poll_and_relay(
    router: &Arc<Mutex<Router>>,
    client: &reqwest::Client,
    server_url: &str,
    our_x25519: &[u8; 32],
    identity: &Arc<Identity>,
    quiet: bool,
) {
    relay_outbound(router, client, server_url).await;
    fetch_inbound(router, client, server_url, our_x25519, identity, quiet).await;
}

// ── Relay outbound ────────────────────────────────────────────────────────────

/// Submit all undelivered bundles to the rendezvous server.
///
/// Phase 1: we submit everything. Phase 3 will filter by transport so we
/// skip bundles already synced over BLE.
async fn relay_outbound(router: &Arc<Mutex<Router>>, client: &reqwest::Client, server_url: &str) {
    // Collect bundles while holding the lock, then release it before
    // doing any I/O. You must never hold a Mutex lock across an .await —
    // if the task is suspended while holding the lock, no other task can
    // ever acquire it, which is a deadlock.
    let bundles: Vec<Bundle> = {
        let r = router.lock().unwrap();
        r.outbound_bundles().unwrap_or_default()
    }; // <-- lock released here

    for bundle in bundles {
        match bundle.to_bytes() {
            Ok(bytes) => {
                if let Err(e) = relay::submit_bundle(client, server_url, bytes).await {
                    warn!("relay submit failed for {}: {e}", bundle.id);
                } else {
                    info!("submitted bundle {} to relay", bundle.id);
                    // Mark submitted so we don't re-POST on the next tick.
                    // "submitted" = sent to rendezvous server (outbound done).
                    // "delivered" = received by the destination (inbound processed).
                    // These are separate flags — see store migration 002.
                    let r = router.lock().unwrap();
                    if let Err(e) = r.mark_submitted(bundle.id) {
                        warn!("mark_submitted failed for {}: {e}", bundle.id);
                    }
                }
            }
            Err(e) => warn!("failed to serialize bundle {}: {e}", bundle.id),
        }
    }
}

// ── Fetch inbound ─────────────────────────────────────────────────────────────

/// Fetch bundles from our inbox, hand each to the Router, ack delivered ones.
async fn fetch_inbound(
    router: &Arc<Mutex<Router>>,
    client: &reqwest::Client,
    server_url: &str,
    our_x25519: &[u8; 32],
    identity: &Arc<Identity>,
    quiet: bool,
) {
    info!("polling inbox...");

    let blobs = match relay::fetch_inbox(client, server_url, our_x25519).await {
        Ok(b) => b,
        Err(e) => {
            warn!("inbox fetch failed: {e}");
            return;
        }
    };

    if blobs.is_empty() {
        info!("inbox empty");
        return;
    }

    info!("received {} bundle(s) from relay", blobs.len());

    let now = unix_now();

    for raw in &blobs {
        let bundle = match Bundle::from_bytes(raw) {
            Ok(b) => b,
            Err(e) => {
                warn!("failed to deserialize bundle: {e}");
                continue;
            }
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
                handle_actions(actions, router, identity, quiet);
                // Mark delivered locally so we don't process it again.
                {
                    let r = router.lock().unwrap();
                    if let Err(e) = r.mark_delivered(bundle_id) {
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

fn handle_actions(
    actions: Vec<Action>,
    router: &Arc<Mutex<Router>>,
    identity: &Identity,
    quiet: bool,
) {
    for action in actions {
        match action {
            Action::NotifyUser { bundle_id } => {
                // Fetch the bundle from the store so we can decrypt its payload.
                //
                // Lock scope: we acquire the lock, do the synchronous store
                // lookup, then immediately release it by letting `r` drop at
                // the closing `}`. This keeps the lock held for the minimum
                // time necessary — no lock is held during the decrypt or print.
                let bundle = {
                    let r = router.lock().unwrap();
                    r.get_bundle(bundle_id)
                };

                match bundle {
                    Err(e) => {
                        warn!("could not fetch bundle {bundle_id} for display: {e}");
                    }
                    Ok(None) => {
                        warn!("bundle {bundle_id} not found in store (already expired?)");
                    }
                    Ok(Some(b)) => {
                        // bundle.origin_x25519 is the sender's X25519 pubkey —
                        // distinct from bundle.origin (Ed25519). The recipient
                        // uses it to mirror the DH the sender performed during
                        // encrypt(). These are NOT interchangeable — see ADR-006.
                        match crypto::decrypt(identity, &b.origin_x25519, &b.payload) {
                            Ok(plaintext) => {
                                let text = String::from_utf8_lossy(&plaintext);
                                // Always print messages regardless of --quiet.
                                // This is the whole point of --quiet: silence
                                // tracing noise but keep message output visible.
                                println!("📨  from {} | {}", hex::encode(&b.origin[..8]), text);
                                // Mark displayed so `ripple status` unread count clears.
                                let r = router.lock().unwrap();
                                if let Err(e) = r.mark_displayed(bundle_id) {
                                    warn!("mark_displayed failed for {bundle_id}: {e}");
                                }
                            }
                            Err(e) => {
                                // Decryption failure is unusual but not fatal.
                                // Could mean the bundle was addressed to a different
                                // X25519 key than ours, or data is corrupt.
                                warn!("decryption failed for bundle {bundle_id}: {e}");
                            }
                        }
                    }
                }
            }
            Action::ForwardBundle {
                peer_pubkey,
                bundle_id,
            } => {
                if !quiet {
                    info!(
                        "queued forward of {} to peer {}",
                        bundle_id,
                        hex::encode(&peer_pubkey[..8])
                    );
                }
            }
            Action::UpdateSharedState { key, value: _ } => {
                if !quiet {
                    info!("shared state update: {key}");
                }
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
