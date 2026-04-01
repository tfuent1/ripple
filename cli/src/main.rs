//! Ripple CLI — Milestone 1.7
//!
//! Commands:
//!   ripple daemon [--server <url>]        — start the mesh daemon
//!   ripple send <message>                 — queue a broadcast bundle
//!   ripple send --to <pubkey_hex> <msg>   — queue an encrypted direct bundle
//!   ripple status                         — show identity and queue info
//!   ripple peers                          — list recently encountered peers

mod commands {
    pub mod daemon;
    pub mod peers;
    pub mod send;
    pub mod status;
}
mod identity;
mod relay;

use clap::{Parser, Subcommand};
use ripple_core::routing::Router;
use ripple_core::store::Store;

/// Ripple mesh communication daemon and tooling.
#[derive(Parser)]
#[command(name = "ripple", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the mesh daemon (runs until killed).
    Daemon {
        /// Rendezvous server URL.
        #[arg(long, default_value = "http://localhost:8080")]
        server: String,

        /// Suppress tracing log output. Message lines are always shown.
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },

    /// Queue a bundle for sending.
    Send {
        /// Recipient's X25519 pubkey (hex). Omit for broadcast.
        #[arg(long)]
        to: Option<String>,

        /// Message text.
        message: String,
    },

    /// Show identity and queue info.
    Status,

    /// List recently encountered peers.
    Peers,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // All commands need an identity and a store/router.
    let identity = identity::load_or_create()?;

    let db_path = {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        format!("{home}/.ripple/mesh.db")
    };

    let store  = Store::new(&db_path)?;
    let router = Router::new(store, identity.x25519_public_key());

    match cli.command {
        Command::Daemon { server, quiet } => {
            if !quiet {
                tracing_subscriber::fmt::init();
            }
            // The daemon is async — we need to start the tokio runtime.
            // `tokio::runtime::Runtime::new()` is the manual equivalent of
            // `#[tokio::main]` — we use the manual form here because `main()`
            // itself isn't async (it returns `anyhow::Result<()>`).
            tokio::runtime::Runtime::new()?
                .block_on(commands::daemon::run(router, identity, server, quiet));
        }

        Command::Send { to, message } => {
            let mut router = router;
            commands::send::run(&mut router, &identity, &message, to.as_deref())?;
        }

        Command::Status => {
            commands::status::run(&router, &identity)?;
        }

        Command::Peers => {
            commands::peers::run(&router)?;
        }
    }

    Ok(())
}
