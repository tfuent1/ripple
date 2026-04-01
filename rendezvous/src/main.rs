//! Ripple rendezvous server — Milestone 1.7 stub.
//!
//! Three endpoints:
//!   POST   /bundle          — store a bundle for its destination
//!   GET    /inbox/:pubkey   — fetch all bundles for a pubkey (base64 JSON array)
//!   DELETE /bundle/:id      — acknowledge delivery, remove bundle
//!
//! This is the minimum needed to unblock end-to-end CLI testing.
//! Full hardening (rate limiting, auth, separate DB file) is Milestone 1.8.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
    Router,
};
use ripple_core::bundle::{Bundle, Destination};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tracing::info;

// ── Shared state ──────────────────────────────────────────────────────────────

/// The server's shared state — just a SQLite connection behind a Mutex.
///
/// **Rust note on `Arc<Mutex<T>>`:**
/// axum handlers run concurrently (multiple requests at the same time), so
/// they need to share the DB connection safely. `Mutex<T>` lets only one
/// handler touch the connection at a time. `Arc` ("Atomic Reference Count")
/// lets multiple handlers each hold a reference to the *same* Mutex without
/// copying it. Think of `Arc` as Rust's thread-safe reference counting —
/// like PHP's garbage collector, but explicit and zero-overhead.
type AppState = Arc<Mutex<Connection>>;

// ── Schema ────────────────────────────────────────────────────────────────────

fn init_db(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS bundles (
            id          TEXT    PRIMARY KEY,
            dest_pubkey TEXT    NOT NULL,
            raw         BLOB    NOT NULL,
            expires_at  INTEGER,
            created_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_dest ON bundles(dest_pubkey);",
    )
    .expect("failed to create schema");
}

// ── POST /bundle ──────────────────────────────────────────────────────────────

/// Accept a raw MessagePack bundle and store it.
///
/// The server is intentionally dumb — it doesn't parse the bundle contents.
/// It reads just enough to route: the destination pubkey and expiry time.
/// Everything else is opaque bytes.
async fn post_bundle(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> StatusCode {
    // Use Bundle::from_bytes — same deserialization path as the CLI and core.
    // No partial envelope needed; we have the real type.
    let bundle = match Bundle::from_bytes(&body) {
        Ok(b)  => b,
        Err(e) => {
            tracing::warn!("failed to parse bundle: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    let dest_pubkey_hex = match &bundle.destination {
        Destination::Peer(pk) => hex::encode(pk),
        Destination::Broadcast => "broadcast".to_string(),
    };

    let conn = state.lock().unwrap();
    let result = conn.execute(
        "INSERT OR IGNORE INTO bundles (id, dest_pubkey, raw, expires_at, created_at)
         VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
        params![
            bundle.id.to_string(),
            dest_pubkey_hex,
            body.as_ref(),
            bundle.expires_at,
        ],
    );

    match result {
        Ok(_)  => {
            info!("stored bundle {}", bundle.id);
            StatusCode::CREATED
        }
        Err(e) => {
            tracing::error!("db error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

// ── GET /inbox/:pubkey ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct InboxResponse {
    bundles: Vec<String>, // base64-encoded raw bundle bytes
}

/// Return all pending bundles for a pubkey.
/// The client gets a JSON array of base64-encoded bundle blobs.
async fn get_inbox(
    State(state): State<AppState>,
    Path(pubkey_hex): Path<String>,
) -> Result<Json<InboxResponse>, StatusCode> {
    let conn = state.lock().unwrap();

    // Expire bundles while we're here — keep the inbox clean.
    conn.execute(
        "DELETE FROM bundles WHERE expires_at IS NOT NULL AND expires_at <= strftime('%s','now')",
        [],
    ).ok();

    let mut stmt = conn
        .prepare("SELECT raw FROM bundles WHERE dest_pubkey = ?1")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let rows: Vec<String> = stmt
        .query_map(params![pubkey_hex], |row| {
            let raw: Vec<u8> = row.get(0)?;
            Ok(base64_encode(&raw))
        })
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .filter_map(|r| r.ok())
        .collect();

    Ok(Json(InboxResponse { bundles: rows }))
}

// ── DELETE /bundle/:id ────────────────────────────────────────────────────────

/// Acknowledge delivery — remove a bundle by ID.
async fn delete_bundle(
    State(state): State<AppState>,
    Path(bundle_id): Path<String>,
) -> StatusCode {
    let conn = state.lock().unwrap();
    match conn.execute("DELETE FROM bundles WHERE id = ?1", params![bundle_id]) {
        Ok(_)  => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

// ── base64 helper ─────────────────────────────────────────────────────────────

fn base64_encode(bytes: &[u8]) -> String {
    // stdlib doesn't have base64 — we use a simple alphabet encode.
    // In Milestone 1.8 we'll add the `base64` crate properly.
    // For now this is enough to unblock the CLI test.
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let b0 = bytes[i] as usize;
        let b1 = if i + 1 < bytes.len() { bytes[i + 1] as usize } else { 0 };
        let b2 = if i + 2 < bytes.len() { bytes[i + 2] as usize } else { 0 };
        out.push(ALPHABET[b0 >> 2] as char);
        out.push(ALPHABET[((b0 & 0x3) << 4) | (b1 >> 4)] as char);
        if i + 1 < bytes.len() {
            out.push(ALPHABET[((b1 & 0xf) << 2) | (b2 >> 6)] as char);
        } else {
            out.push('=');
        }
        if i + 2 < bytes.len() {
            out.push(ALPHABET[b2 & 0x3f] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let conn = Connection::open_in_memory().expect("failed to open SQLite");
    init_db(&conn);
    let state: AppState = Arc::new(Mutex::new(conn));

    let app = Router::new()
        .route("/bundle",        post(post_bundle))
        .route("/inbox/:pubkey", get(get_inbox))
        .route("/bundle/:id",    delete(delete_bundle))
        .with_state(state);

    let addr = "0.0.0.0:8080";
    info!("rendezvous server listening on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
