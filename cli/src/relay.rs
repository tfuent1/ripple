//! Internet relay transport — HTTP polling and bundle submission.
//!
//! The rendezvous server is a simple store-and-forward relay. We:
//!   - POST our outbound bundles to /bundle
//!   - GET /inbox/:our_x25519_pubkey_hex every 30 seconds
//!   - DELETE /bundle/:id after we've processed each received bundle

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

/// Submit a bundle to the rendezvous server.
///
/// `bundle_bytes` is the raw MessagePack bundle (from `bundle.to_bytes()`).
///
/// **Rust async note:** `async fn` here means "this function may pause while
/// waiting for the HTTP response". The caller must `.await` it. The function
/// doesn't block any thread — tokio suspends it and runs other tasks until
/// the server responds.
pub async fn submit_bundle(
    client: &reqwest::Client,
    server_url: &str,
    bundle_bytes: Vec<u8>,
) -> Result<(), String> {
    let url = format!("{server_url}/bundle");

    let response = client
        .post(&url)
        .body(bundle_bytes)
        .header("Content-Type", "application/octet-stream")
        .send()
        .await
        .map_err(|e| format!("HTTP error submitting bundle: {e}"))?;

    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("server returned {}", response.status()))
    }
}

/// Fetch all pending bundles from our inbox on the rendezvous server.
///
/// Returns a list of raw MessagePack bundle bytes (one per bundle).
/// The caller (daemon) is responsible for passing each to the Router.
pub async fn fetch_inbox(
    client: &reqwest::Client,
    server_url: &str,
    our_x25519_pubkey: &[u8; 32],
) -> Result<Vec<Vec<u8>>, String> {
    let pubkey_hex = hex::encode(our_x25519_pubkey);
    let url = format!("{server_url}/inbox/{pubkey_hex}");

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP error fetching inbox: {e}"))?;

    if !response.status().is_success() {
        return Err(format!("server returned {}", response.status()));
    }

    // The server returns `{ "bundles": ["base64...", "base64...", ...] }`.
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse inbox response: {e}"))?;

    let bundles = body["bundles"]
        .as_array()
        .ok_or("missing bundles field")?
        .iter()
        .filter_map(|v| {
            let b64 = v.as_str()?;
            B64.decode(b64).ok()
        })
        .collect();

    Ok(bundles)
}

/// Acknowledge delivery of a bundle (removes it from the relay server).
pub async fn ack_bundle(
    client: &reqwest::Client,
    server_url: &str,
    bundle_id: uuid::Uuid,
) -> Result<(), String> {
    let url = format!("{server_url}/bundle/{bundle_id}");
    client
        .delete(&url)
        .send()
        .await
        .map_err(|e| format!("HTTP error acking bundle: {e}"))?;
    Ok(())
}

