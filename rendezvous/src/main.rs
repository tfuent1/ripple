//! Ripple rendezvous server — Milestone 1.8.
//!
//! Usage:
//!   rendezvous [--port <PORT>] [--db <PATH>] [--max-bundle-kb <KB>]
//!
//! Defaults: port 8080, db ~/.ripple/rendezvous.db, max bundle 64 KB.

mod db;
mod server;

use clap::Parser;
use db::Db;
use server::AppState;
use std::net::SocketAddr;
use tracing::info;

// ── CLI flags ─────────────────────────────────────────────────────────────────

/// Ripple rendezvous / relay server.
///
/// **Rust note — `#[derive(Parser)]`:**
/// clap's derive macro reads the struct fields and their types to build the
/// argument parser automatically. `Option<T>` fields become optional flags.
/// The `///` doc comments become the `--help` text. Same pattern as the CLI
/// crate — one struct, zero boilerplate.
#[derive(Parser, Debug)]
#[command(name = "rendezvous", about = "Ripple rendezvous server")]
struct Args {
    /// TCP port to listen on.
    #[arg(long, default_value_t = 8080)]
    port: u16,

    /// Path to the SQLite database file.
    /// Defaults to ~/.ripple/rendezvous.db
    #[arg(long)]
    db: Option<String>,

    /// Maximum accepted bundle size in kilobytes.
    #[arg(long, default_value_t = 64)]
    max_bundle_kb: usize,
}

// ── main ──────────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    // Resolve DB path: flag > default (~/.ripple/rendezvous.db).
    let db_path = match args.db {
        Some(p) => p,
        None => {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            let dir = format!("{home}/.ripple");
            std::fs::create_dir_all(&dir).expect("failed to create ~/.ripple");
            format!("{dir}/rendezvous.db")
        }
    };

    info!("opening database at {db_path}");
    let db = Db::open(&db_path).expect("failed to open database");

    let state = AppState::new(db, args.max_bundle_kb * 1024);
    let router = server::build_router(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], args.port));
    info!("rendezvous server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind port");

    // `into_make_service_with_connect_info` is what gives handlers access to
    // the caller's IP address via `ConnectInfo<SocketAddr>`. If we used plain
    // `into_make_service()` instead, the `ConnectInfo` extractor in
    // `post_bundle` would panic at runtime.
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("server error");

    info!("server shut down cleanly");
}

// ── Graceful shutdown ─────────────────────────────────────────────────────────

/// Resolves when SIGINT (Ctrl-C) or SIGTERM is received.
///
/// **Rust note — `async fn` returning a future:**
/// `with_graceful_shutdown` takes a future that resolves when the server
/// should stop accepting new connections and drain in-flight requests.
/// We pass this function's output — axum holds it and `.await`s it
/// internally. When it resolves, axum finishes any active requests and
/// then returns from `serve(...)`.
///
/// `tokio::signal::ctrl_c()` handles SIGINT (Ctrl-C).
/// `unix::signal(SIGTERM)` handles the signal sent by `docker stop` and
/// most process supervisors.
async fn shutdown_signal() {
    use tokio::signal;

    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl-C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    // On non-Unix platforms (Windows) we only handle Ctrl-C.
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    // `tokio::select!` races multiple futures and resolves when the first
    // one completes — like Promise.race() in JS. Either signal triggers
    // a clean shutdown; the other is dropped.
    tokio::select! {
        _ = ctrl_c   => { info!("received Ctrl-C, shutting down"); },
        _ = terminate => { info!("received SIGTERM, shutting down"); },
    }
}
