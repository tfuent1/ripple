//! Internet relay transport — HTTP polling and bundle submission.
//!
//! The rendezvous server is a simple store-and-forward relay. We:
//!   - POST our outbound bundles to /bundle
//!   - GET /inbox/:our_x25519_pubkey_hex every 30 seconds
//!   - DELETE /bundle/:id after we've processed each received bundle

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
            base64_decode(b64).ok()
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

// ── base64 decode helper ──────────────────────────────────────────────────────

/// Minimal base64 decode (no external dep needed for milestone 1.7).
fn base64_decode(input: &str) -> Result<Vec<u8>, ()> {
    let input = input.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);

    fn val(c: u8) -> Option<u8> {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        ALPHABET.iter().position(|&a| a == c).map(|i| i as u8)
    }

    let mut i = 0;
    while i + 1 < input.len() {
        let b0 = val(input[i]).ok_or(())?;
        let b1 = val(input[i + 1]).ok_or(())?;
        out.push((b0 << 2) | (b1 >> 4));
        if i + 2 < input.len() {
            let b2 = val(input[i + 2]).ok_or(())?;
            out.push(((b1 & 0xf) << 4) | (b2 >> 2));
            if i + 3 < input.len() {
                let b3 = val(input[i + 3]).ok_or(())?;
                out.push(((b2 & 0x3) << 6) | b3);
            }
        }
        i += 4;
    }
    Ok(out)
}
