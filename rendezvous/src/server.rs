//! Axum request handlers and shared server state.
//!
//! The handler functions here are thin — they delegate all DB work to `db::Db`
//! and focus only on HTTP concerns: parsing the request, calling the DB,
//! returning the right status code.

use crate::db::{Db, DbError};
use axum::{
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    response::Json,
    routing::{delete, get, post},
    Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde::Serialize;
use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tower_http::limit::RequestBodyLimitLayer;
use tracing::info;

// ── Constants ─────────────────────────────────────────────────────────────────

/// How many bundle submissions one IP may make per window.
const RATE_LIMIT_MAX: u32 = 60;

/// The rolling window for rate limiting.
const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(60);

// ── Shared state ──────────────────────────────────────────────────────────────

/// Per-IP submission counter for rate limiting.
///
/// We keep a `HashMap` of `(count, window_start)` keyed by IP address.
/// On each POST /bundle we check whether the window has expired (reset if so)
/// and whether the count is under the limit.
///
/// **Why not use tower's built-in rate limiter?**
/// `tower::limit::RateLimit` is global — it limits total requests across all
/// callers. We want per-IP limits so one noisy client can't block others.
/// A `HashMap<IpAddr, (u32, Instant)>` behind a `Mutex` is the simplest
/// correct solution for Phase 1 traffic volumes.
#[derive(Default)]
struct RateLimiter {
    counters: HashMap<std::net::IpAddr, (u32, Instant)>,
}

impl RateLimiter {
    /// Returns `true` if the request should be allowed, `false` if rate limited.
    fn check_and_increment(&mut self, ip: std::net::IpAddr) -> bool {
        let now = Instant::now();
        let entry = self.counters.entry(ip).or_insert((0, now));

        // If the window has expired, reset the counter.
        if now.duration_since(entry.1) >= RATE_LIMIT_WINDOW {
            *entry = (0, now);
        }

        if entry.0 >= RATE_LIMIT_MAX {
            return false;
        }

        entry.0 += 1;

        // Evict stale entries on every 100th request to prevent unbounded
        // growth. An attacker sending from rotating IPs would otherwise grow
        // this map without limit. We evict lazily (not on a timer) to avoid
        // needing a background task.
        //
        // `entry.0 % 100 == 0` fires roughly once per window per active IP,
        // which is cheap enough. We retain() only entries whose window has not
        // yet expired — stale ones are gone.
        if entry.0.is_multiple_of(100) {
            self.counters.retain(|_, (_, window_start)| {
                now.duration_since(*window_start) < RATE_LIMIT_WINDOW
            });
        }

        true
    }
}

/// The application state shared across all handlers.
///
/// `Arc` lets multiple handlers (running concurrently on different tokio tasks)
/// each hold a reference to the same underlying data. `Mutex` ensures only one
/// handler touches `db` or `rate_limiter` at a time.
///
/// Notice `db` and `rate_limiter` are in separate `Mutex`es. If they shared
/// one lock, a slow DB query would block rate limit checks for other requests.
/// Fine-grained locking means they're independent.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<Mutex<Db>>,
    rate_limiter: Arc<Mutex<RateLimiter>>,
    pub max_body_bytes: usize,
}

impl AppState {
    pub fn new(db: Db, max_body_bytes: usize) -> Self {
        Self {
            db: Arc::new(Mutex::new(db)),
            rate_limiter: Arc::new(Mutex::new(RateLimiter::default())),
            max_body_bytes,
        }
    }
}

// ── Router ────────────────────────────────────────────────────────────────────

/// Build and return the axum `Router` with all routes and middleware attached.
///
/// **Rust note — returning `Router` from a function:**
/// axum's `Router` is a value like any other. We build it here and return it
/// to `main.rs`, which wires it to a TCP listener. This keeps startup logic
/// in `main.rs` and routing logic here.
///
/// **`RequestBodyLimitLayer`:**
/// This is a tower middleware layer. It wraps the entire router and rejects
/// any request whose body exceeds `MAX_BUNDLE_BYTES` before the handler even
/// runs. The client gets `413 Payload Too Large` automatically.
pub fn build_router(state: AppState) -> Router {
    let limit = state.max_body_bytes;
    Router::new()
        .route("/bundle", post(post_bundle))
        .route("/inbox/:pubkey", get(get_inbox))
        .route("/bundle/:id", delete(delete_bundle))
        .layer(RequestBodyLimitLayer::new(limit))
        .with_state(state)
}

// ── POST /bundle ──────────────────────────────────────────────────────────────

/// Accept a raw MessagePack bundle and store it.
///
/// `ConnectInfo<SocketAddr>` is how axum gives us the caller's IP address.
/// It requires the server to be started with `.into_make_service_with_connect_info::<SocketAddr>()`
/// in `main.rs` — we'll wire that up there.
async fn post_bundle(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    body: axum::body::Bytes,
) -> StatusCode {
    // Rate limit check — acquire lock, check, release immediately.
    // The `{}` block ensures the MutexGuard is dropped before we do any
    // DB work. Never hold two locks at the same time — classic deadlock setup.
    let allowed = {
        let mut rl = state.rate_limiter.lock().unwrap();
        rl.check_and_increment(addr.ip())
    };

    if !allowed {
        tracing::warn!("rate limited: {}", addr.ip());
        return StatusCode::TOO_MANY_REQUESTS;
    }

    let db = state.db.lock().unwrap();
    match db.insert_bundle(&body) {
        Ok(true) => {
            info!("stored bundle from {}", addr.ip());
            StatusCode::CREATED
        }
        Ok(false) => {
            // Duplicate — idempotent, still a success from the client's view.
            StatusCode::CREATED
        }
        Err(DbError::BundleParse(e)) => {
            tracing::warn!("bad bundle from {}: {e}", addr.ip());
            StatusCode::BAD_REQUEST
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

/// Return all pending bundles for a pubkey as base64-encoded JSON.
async fn get_inbox(
    State(state): State<AppState>,
    Path(pubkey_hex): Path<String>,
) -> Result<Json<InboxResponse>, StatusCode> {
    let db = state.db.lock().unwrap();
    let rows = db
        .bundles_for_pubkey(&pubkey_hex)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // `B64.encode` is from the `base64` crate — replaces the hand-rolled encoder.
    // `STANDARD` is the standard alphabet (+/) with padding (=). Same alphabet
    // the hand-rolled encoder used, so existing CLI clients are compatible.
    let bundles = rows.iter().map(|r| B64.encode(r)).collect();
    Ok(Json(InboxResponse { bundles }))
}

// ── DELETE /bundle/:id ────────────────────────────────────────────────────────

/// Acknowledge delivery — remove a bundle by UUID.
async fn delete_bundle(State(state): State<AppState>, Path(bundle_id): Path<String>) -> StatusCode {
    let db = state.db.lock().unwrap();
    match db.delete_bundle(&bundle_id) {
        Ok(_) => StatusCode::OK,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
