//! Internet relay transport — HTTP polling and bundle submission.
//!
//! The rendezvous server is a simple store-and-forward relay. We:
//!   - POST our outbound bundles to /bundle
//!   - GET /inbox/:our_x25519_pubkey_hex every 30 seconds
//!   - DELETE /bundle/:id after we've processed each received bundle

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),

    #[error("server rejected bundle: {0}")]
    ServerError(reqwest::StatusCode),

    #[error("failed to parse inbox response: {0}")]
    Parse(String),
}

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
) -> Result<(), RelayError> {
    let response = client
        .post(format!("{server_url}/bundle"))
        .body(bundle_bytes)
        .header("Content-Type", "application/octet-stream")
        .send()
        .await?; // <-- reqwest::Error auto-converts via #[from]

    if response.status().is_success() {
        Ok(())
    } else {
        Err(RelayError::ServerError(response.status()))
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
) -> Result<Vec<Vec<u8>>, RelayError> {
    let pubkey_hex = hex::encode(our_x25519_pubkey);
    let response = client
        .get(format!("{server_url}/inbox/{pubkey_hex}"))
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(RelayError::ServerError(response.status()));
    }

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| RelayError::Parse(e.to_string()))?;

    let bundles = body["bundles"]
        .as_array()
        .ok_or_else(|| RelayError::Parse("missing bundles field".into()))?
        .iter()
        .filter_map(|v| B64.decode(v.as_str()?).ok())
        .collect();

    Ok(bundles)
}

/// Acknowledge delivery of a bundle (removes it from the relay server).
pub async fn ack_bundle(
    client: &reqwest::Client,
    server_url: &str,
    bundle_id: uuid::Uuid,
) -> Result<(), RelayError> {
    client
        .delete(format!("{server_url}/bundle/{bundle_id}"))
        .send()
        .await?;
    Ok(())
}
