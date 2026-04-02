//! ripple-rendezvous as a library.
//!
//! Exposes `db` and `server` so integration tests in other crates (ripple-cli)
//! can spin up an in-process rendezvous server without spawning a subprocess.
//! The binary entry point remains in `src/main.rs`.

pub mod db;
pub mod server;
